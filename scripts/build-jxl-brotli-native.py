"""2026-11-13 安全更新：以 libjxl 0.13.0／Brotli 1.2.1 重建 ImageMagick 委派 DLL。

只在 work 目錄產出，不自動替換 runtime。
- CORE_RL_brotli_.dll：Brotli v1.2.1（commit db24a98，libjxl v0.13.0 submodule）
- CORE_RL_jpeg-xl_.dll：libjxl v0.13.0（commit 5a7da83）+ jxl_cms(lcms2) + jxl_threads + Highway
  連結既有 CORE_RL_lcms_.dll 與新的 CORE_RL_brotli_.dll，維持原相依圖。
匯出名稱由 0.12.0 基線 DLL 產生 .def，確保與 IM_MOD_RL_jxl_.dll 及原 ABI 名稱集合相同。
MSVC 14.52 /MD Release；不安裝套件、不改 PATH。

用法：python -X utf8 scripts/build-jxl-brotli-native.py
前置：work/claude-round2-20261113/native/libjxl-0.13.0（git clone --branch v0.13.0，
      submodule third_party/brotli 與 third_party/highway）、baseline-0.12.0（舊 DLL 備份）。
"""
from pathlib import Path
import hashlib
import json
import re
import subprocess

root = Path(__file__).resolve().parents[1]
work = root / 'work/claude-round2-20261113/native'
source = work / 'libjxl-0.13.0'
baseline = work / 'baseline-0.12.0/ImageMagick'
build = work / 'build-msvc'
out = work / 'out'
lcms_include = root / 'work/security-followup-20261112/im-core/Dependencies/Dependencies/lcms/include'
vs = Path(r'C:\Program Files\Microsoft Visual Studio\18\Community')
vsdev = vs / 'Common7/Tools/VsDevCmd.bat'
tools = vs / 'VC/Tools/MSVC/14.52.37025/bin/Hostx64/x64'
EXPECTED = {
    '.': '5a7da83447212fe4900fc29ce97430b881caa67e',
    'third_party/brotli': 'db24a98aed38a16de78cc05ca15cdd8dc2a0d92d',
    'third_party/highway': '457c891775a7397bdb0376bb1031e6e027af1c48',
}
build.mkdir(parents=True, exist_ok=True)
out.mkdir(parents=True, exist_ok=True)
log = []


def run(args, label, cwd=build, shell_env=False):
    if shell_env:
        # 在子 cmd 載入 VS 建置環境；呼叫端環境不受影響。
        line = f'call "{vsdev}" -arch=amd64 -host_arch=amd64 -vcvars_ver=14.52 >nul && ' + subprocess.list2cmdline(list(map(str, args)))
        command = f'cmd /d /s /c "{line}"'  # 字串形式，避免 list2cmdline 把引號轉成 \"
    else:
        command = list(map(str, args))
    log.append({'label': label, 'command': command if isinstance(command, str) else subprocess.list2cmdline(command)})
    with (out / f'{label}.log').open('wb') as f:
        result = subprocess.run(command, cwd=cwd, stdout=f, stderr=subprocess.STDOUT)
    print(label, result.returncode, flush=True)
    if result.returncode:
        print((out / f'{label}.log').read_text(errors='replace')[-6000:])
        raise SystemExit(result.returncode)


for path, commit in EXPECTED.items():
    actual = subprocess.check_output(['git', '-C', str(source / path), 'rev-parse', 'HEAD'], text=True).strip()
    if actual != commit:
        raise SystemExit(f'{path} commit {actual} != {commit}')
status = subprocess.check_output(['git', '-C', str(source), 'status', '--porcelain', '--ignore-submodules=none'], text=True)
if status.strip():
    raise SystemExit('libjxl 來源樹有未預期修改：\n' + status)


def exports(dll):
    text = subprocess.check_output([str(tools / 'dumpbin.exe'), '/nologo', '/exports', str(dll)], text=True)
    return re.findall(r'^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]+\s+(\S+)', text, re.M)


def definition(name, symbols):
    path = out / f'{name}.def'
    # Brotli 的 _kBrotli* 是資料表，匯出時標為 DATA，避免產生函式 thunk。
    lines = [f'{s} DATA' if s.startswith('_kBrotli') else s for s in symbols]
    path.write_text(f'LIBRARY {name}.dll\nEXPORTS\n' + '\n'.join(lines) + '\n')
    return path


brotli_exports = exports(baseline / 'CORE_RL_brotli_.dll')
jxl_exports = exports(baseline / 'CORE_RL_jpeg-xl_.dll')
lcms_exports = exports(baseline / 'CORE_RL_lcms_.dll')
assert len(brotli_exports) == 38 and len(jxl_exports) == 115, (len(brotli_exports), len(jxl_exports))
lcms_def = definition('CORE_RL_lcms_', lcms_exports)
run([tools / 'lib.exe', '/nologo', '/machine:x64', f'/def:{lcms_def}', f'/out:{out / "CORE_RL_lcms_.lib"}'], 'lcms-import', cwd=out)

# 2026-11-13 RC：以 MSVC 的 /pathmap（需搭配 /experimental:deterministic）把原始碼與建置路徑
# 對應成中性名稱，避免 __FILE__ 在 DLL 內嵌入本機開發路徑。其餘旗標與 CMake 4.x 的 MSVC 預設相同。
PATHMAP = f'/experimental:deterministic /pathmap:{source}=libjxl-0.13.0 /pathmap:{build}=build /pathmap:{root}=/build-user'
run(['cmake', '-G', 'Ninja', '-S', source, '-B', build,
     '-DCMAKE_BUILD_TYPE=Release',
     f'-DCMAKE_C_FLAGS=/DWIN32 /D_WINDOWS {PATHMAP}',
     f'-DCMAKE_CXX_FLAGS=/DWIN32 /D_WINDOWS /EHsc {PATHMAP}',
     '-DCMAKE_C_COMPILER=cl', '-DCMAKE_CXX_COMPILER=cl',
     '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL',
     '-DBUILD_SHARED_LIBS=OFF', '-DBUILD_TESTING=OFF',
     '-DJPEGXL_ENABLE_TOOLS=OFF', '-DJPEGXL_ENABLE_JPEGLI=OFF', '-DJPEGXL_ENABLE_DOXYGEN=OFF',
     '-DJPEGXL_ENABLE_MANPAGES=OFF', '-DJPEGXL_ENABLE_BENCHMARK=OFF', '-DJPEGXL_ENABLE_EXAMPLES=OFF',
     '-DJPEGXL_ENABLE_JNI=OFF', '-DJPEGXL_ENABLE_SJPEG=OFF', '-DJPEGXL_ENABLE_OPENEXR=OFF',
     '-DJPEGXL_ENABLE_PLUGINS=OFF', '-DJPEGXL_ENABLE_VIEWERS=OFF', '-DJPEGXL_ENABLE_DEVTOOLS=OFF',
     '-DJPEGXL_ENABLE_FUZZERS=OFF', '-DJPEGXL_BUNDLE_LIBPNG=OFF',
     '-DJPEGXL_ENABLE_SKCMS=OFF', '-DJPEGXL_FORCE_SYSTEM_LCMS2=ON',
     f'-DLCMS2_INCLUDE_DIR={lcms_include}', f'-DLCMS2_LIBRARY={out / "CORE_RL_lcms_.lib"}',
     '-DJPEGXL_FORCE_SYSTEM_HWY=OFF', '-DJPEGXL_FORCE_SYSTEM_BROTLI=OFF',
     '-DJPEGXL_ENABLE_TRANSCODE_JPEG=ON', '-DJPEGXL_ENABLE_BOXES=ON'], 'cmake-configure', shell_env=True)
run(['cmake', '--build', build, '--target', 'jxl', 'jxl_cms', 'jxl_threads', 'brotlidec', 'brotlienc', 'brotlicommon', 'hwy'], 'cmake-build', shell_env=True)


def find(name):
    hits = sorted(build.rglob(name))
    if len(hits) != 1:
        raise SystemExit(f'{name}: {hits}')
    return hits[0]


libs = {n: find(f'{n}.lib') for n in ['jxl', 'jxl_cms', 'jxl_threads', 'hwy', 'brotlicommon', 'brotlidec', 'brotlienc']}


def resource(name, version, description):
    numbers = ','.join((version.split('.') + ['0', '0', '0'])[:4])
    rc = out / f'{name}.rc'
    rc.write_text(f'''1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS 0x40004
FILETYPE 0x2
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "{description}"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "{name}.dll"
      VALUE "Comments", "IMB-C 2026-11-13 security rebuild"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
''')
    run(['rc.exe', '/nologo', f'/fo{out / (name + ".res")}', rc], f'{name}-rc', cwd=out, shell_env=True)
    return out / f'{name}.res'


brotli_def = definition('CORE_RL_brotli_', brotli_exports)
run([tools / 'link.exe', '/nologo', '/DLL', '/MACHINE:X64', '/OPT:REF', '/OPT:ICF', '/NXCOMPAT', '/DYNAMICBASE', '/HIGHENTROPYVA',
     f'/DEF:{brotli_def}', f'/OUT:{out / "CORE_RL_brotli_.dll"}', f'/IMPLIB:{out / "CORE_RL_brotli_.lib"}',
     libs['brotlienc'], libs['brotlidec'], libs['brotlicommon'], resource('CORE_RL_brotli_', '1.2.1', 'Brotli 1.2.1'),
     'kernel32.lib'], 'brotli-link', cwd=out, shell_env=True)

jxl_def = definition('CORE_RL_jpeg-xl_', jxl_exports)
run([tools / 'link.exe', '/nologo', '/DLL', '/MACHINE:X64', '/OPT:REF', '/OPT:ICF', '/NXCOMPAT', '/DYNAMICBASE', '/HIGHENTROPYVA',
     f'/DEF:{jxl_def}', f'/OUT:{out / "CORE_RL_jpeg-xl_.dll"}', f'/IMPLIB:{out / "CORE_RL_jpeg-xl_.lib"}',
     libs['jxl'], libs['jxl_cms'], libs['jxl_threads'], libs['hwy'],
     out / 'CORE_RL_brotli_.lib', out / 'CORE_RL_lcms_.lib', resource('CORE_RL_jpeg-xl_', '0.13.0', 'libjxl 0.13.0'),
     'kernel32.lib'], 'jxl-link', cwd=out, shell_env=True)

def assert_no_local_paths(path):
    data = path.read_bytes().lower()
    for text in {str(root), str(root).replace('\\', '/'), str(work), str(source)}:
        for needle in (text.lower().encode('utf-8'), text.lower().encode('utf-16-le')):
            if needle in data:
                raise SystemExit(f'{path.name} 仍內嵌本機路徑 {text}')


for name in ['CORE_RL_brotli_.dll', 'CORE_RL_jpeg-xl_.dll']:
    assert_no_local_paths(out / name)
    new = exports(out / name)
    old = brotli_exports if 'brotli' in name else jxl_exports
    if sorted(new) != sorted(old):
        raise SystemExit(f'{name} 匯出集合不同：{sorted(set(old) ^ set(new))}')
result = {
    'sources': EXPECTED,
    'cmake_cache': {k: v for k, v in (line.split('=', 1) for line in (build / 'CMakeCache.txt').read_text(errors='replace').splitlines()
                                       if '=' in line and line.split(':', 1)[0] in {'CMAKE_CXX_COMPILER', 'CMAKE_CXX_FLAGS_RELEASE', 'CMAKE_C_FLAGS_RELEASE', 'JPEGXL_ENABLE_SKCMS', 'JPEGXL_FORCE_SYSTEM_LCMS2', 'JPEGXL_ENABLE_TRANSCODE_JPEG', 'JPEGXL_ENABLE_BOXES', 'CMAKE_MSVC_RUNTIME_LIBRARY'})},
    'outputs': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in [out / 'CORE_RL_brotli_.dll', out / 'CORE_RL_jpeg-xl_.dll']},
    'commands': log,
}
(out / 'build-result.json').write_text(json.dumps(result, indent=2, ensure_ascii=False))
print(json.dumps(result['outputs'], indent=2))
