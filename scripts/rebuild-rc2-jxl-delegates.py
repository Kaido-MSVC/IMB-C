"""Build the RC2 ImageMagick Brotli and JPEG XL delegate DLLs from fixed source."""

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import zipfile


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def extract_snapshot(path, dest):
    with zipfile.ZipFile(path) as bundle:
        seen = set()
        for item in bundle.infolist():
            name = PurePosixPath(item.filename)
            if (name.is_absolute() or ".." in name.parts or ":" in item.filename
                    or "\\" in item.filename or (item.external_attr >> 28) == 10
                    or item.filename.casefold() in seen):
                raise ValueError("Unsafe snapshot entry: " + item.filename)
            seen.add(item.filename.casefold())
            if item.is_dir():
                continue
            target = dest.joinpath(*name.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.open(item) as src, target.open("xb") as out_file:
                shutil.copyfileobj(src, out_file)


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--snapshot", type=Path, required=True)
parser.add_argument("--imagemagick-source", type=Path, required=True)
parser.add_argument("--portable", type=Path, required=True)
parser.add_argument("--work", type=Path, required=True, help="A new directory at a short local path")
parser.add_argument("--vs", type=Path, required=True, help="Visual Studio 2026 installation directory")
args = parser.parse_args()

work = args.work.resolve()
root = work
source = work / "libjxl-0.13.0"
baseline = args.portable.resolve() / "ImageMagick"
build = work / "build"
out = work / "out"
lcms_include = work / "lcms-include"
vs = args.vs.resolve()
vsdev = vs / "Common7/Tools/VsDevCmd.bat"
tools = vs / "VC/Tools/MSVC/14.52.37025/bin/Hostx64/x64"
EXPECTED = {
    ".": "5a7da83447212fe4900fc29ce97430b881caa67e",
    "third_party/brotli": "db24a98aed38a16de78cc05ca15cdd8dc2a0d92d",
    "third_party/highway": "457c891775a7397bdb0376bb1031e6e027af1c48",
}
reference_dlls = {
    "CORE_RL_brotli_.dll": "502603493562f32aaf777a358b0b13ca567531f33dbc071a7d0a3f4d10311121",
    "CORE_RL_jpeg-xl_.dll": "bc8845288150afc48e9e4fce242f1ca1858e77529ea7da0981b3f31f3632908c",
    "CORE_RL_lcms_.dll": "a23f62712e5348c219ec08863902c7cf2c900ec48dd8bcae7a4b601607a09302",
}
for filename, digest in reference_dlls.items():
    if sha(baseline / filename) != digest:
        raise ValueError("Unexpected reference DLL: " + filename)
if sha(args.snapshot) != "9ae1075545921c140a0cdbd58b584e60c8a99ce63835bd068a78a87e5a91c638":
    raise ValueError("Unexpected libjxl / Brotli / Highway source archive")
if sha(args.imagemagick_source) != "cd9452dd3121396169cfb74312dd92715fd2edeaabf83c9a8efcdf87b0f45915":
    raise ValueError("Unexpected ImageMagick source archive")
if not vsdev.is_file() or not (tools / "cl.exe").is_file():
    raise ValueError("Visual Studio 2026 with MSVC 14.52.37025 is required")

work.mkdir(parents=True)
extract_snapshot(args.snapshot, work)
if json.loads((work / "SOURCE-PINS.json").read_bytes()) != EXPECTED:
    raise ValueError("Unexpected source pins")
build.mkdir()
out.mkdir()
lcms_include.mkdir()
with zipfile.ZipFile(args.imagemagick_source) as archive:
    for name in ("lcms2.h", "lcms2_plugin.h"):
        path = "Dependencies/Dependencies/lcms/include/" + name
        (lcms_include / name).write_bytes(archive.read(path))

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
     '-DCMAKE_BUILD_TYPE=Release', '-DCMAKE_TRY_COMPILE_CONFIGURATION=Release',
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
    for text in {str(work), str(work).replace('\\', '/'), str(source)}:
        for needle in (text.lower().encode('utf-8'), text.lower().encode('utf-16-le')):
            if needle in data:
                raise SystemExit(f'{path.name} 仍內嵌本機路徑 {text}')


for name in ['CORE_RL_brotli_.dll', 'CORE_RL_jpeg-xl_.dll']:
    assert_no_local_paths(out / name)
    new = exports(out / name)
    old = brotli_exports if 'brotli' in name else jxl_exports
    if sorted(new) != sorted(old) or len(new) != len(set(new)):
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
