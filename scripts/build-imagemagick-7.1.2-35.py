"""2026-11-13: rebuild ImageMagick 7.1.2-35 (Q16-HDRI x64 dynamic); outputs only to work/, never replaces the runtime automatically.

Sources and settings:
- Official ImageMagick-7.1.2-35-Windows.7z (SHA-256 / SLSA attestation verified; source commit = tag 7.1.2-35 5d1bcc55f).
  Expected extraction: work/claude-round3-20261113/im-35-src, with the official Configure already run once
  `Configure.Release.x64.exe /noWizard /VS2026 /x64 /dynamic /hdri /Q16` to produce version.h,
  package.version.h, and the ProjectFiles (official source lists, compile settings, resource files).
- Only ImageMagick's own code is rebuilt: MagickCore, MagickWand, magick.exe, and the 10 whitelisted coders.
  Third-party dependency DLLs are left unchanged (except the libheif 1.23.7 / libjxl 0.13.0 / Brotli 1.2.1 already updated in earlier rounds);
  their headers come from the 7.1.2-34 set matching the shipped DLLs (libtiff 4.7.2 / liblzma 5.8.4 are identical versions between -34/-35, taken from -35).
- IMB-C changes: the core isolation patch (configure/module search limited to the directory of the loaded DLL), and the psd/heic coder and CLI notices
  (same logic as earlier rounds; the corresponding -34 and -35 original files are verified byte-identical before applying).
- magick-baseconfig.h: the option-derived part of the official Configure output must match the -34 header exactly; the delegate section keeps IMB-C's existing subset.

Usage: python -X utf8 scripts/build-imagemagick-7.1.2-35.py
"""
from pathlib import Path
import hashlib
import json
import os
import re
import shutil
import subprocess
import xml.etree.ElementTree as ET
import zipfile

root = Path(__file__).resolve().parents[1]
work = root / 'work/claude-round3-20261113'
src35 = work / 'im-35-src'
build = work / 'im-35-build'
out = build / 'out'
bsrc = build / 'src'
im = bsrc / 'ImageMagick'
prev = root / 'work/security-followup-20261112'
deps34 = prev / 'im-core/Dependencies/Dependencies'
deps35 = src35 / 'Dependencies/Dependencies'
heif = prev / 'heif-install'
jxl = root / 'work/claude-round2-20261113/native'
runtime = root / 'target/release/ImageMagick'
selected34 = root / 'target/release/licenses/ImageMagick-7.1.2-34-selected-corresponding-source.zip'
vs = Path(r'C:\Program Files\Microsoft Visual Studio\18\Community')
vsdev = vs / 'Common7/Tools/VsDevCmd.bat'
tools = vs / 'VC/Tools/MSVC/14.52.37025/bin/Hostx64/x64'
TAG_COMMIT = '5d1bcc55f2c83bc2ff802118b7369756610f3ad4'
ARCHIVE_SHA = '778bc4d5de0a8ae055284e85fd7dfff75dd44ec14c010cdf8dad5ab05f6a89d4'
log = []


def run(args, label, cwd=None):
    args = list(map(str, args))
    if len(subprocess.list2cmdline(args)) > 6000:
        # cmd length limit: cl/link read the remaining arguments from a response file.
        rsp = out / f'{label}.rsp'
        rsp.write_text('\n'.join(subprocess.list2cmdline([a]) for a in args[1:]) + '\n', encoding='utf-8')
        args = [args[0], f'@{rsp}']
    line = f'call "{vsdev}" -arch=amd64 -host_arch=amd64 -vcvars_ver=14.52 >nul && ' + subprocess.list2cmdline(args)
    command = f'cmd /d /s /c "{line}"'
    log.append({'label': label, 'command': command})
    (out / 'commands.json').write_text(json.dumps(log, indent=2, ensure_ascii=False), encoding='utf-8')
    with (out / f'{label}.log').open('wb') as f:
        result = subprocess.run(command, cwd=cwd or out, stdout=f, stderr=subprocess.STDOUT)
    print(label, result.returncode, flush=True)
    if result.returncode:
        print((out / f'{label}.log').read_text(errors='replace')[-6000:])
        raise SystemExit(result.returncode)


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


# ---- 0. Source checks -------------------------------------------------------------
archive = work / 'im-src-download/ImageMagick-7.1.2-35-Windows.7z'
assert sha(archive) == ARCHIVE_SHA, 'Windows.7z digest mismatch'
head = subprocess.check_output(['git', '-C', str(src35 / 'ImageMagick'), 'rev-parse', 'HEAD'], text=True).strip()
assert head == TAG_COMMIT, head
version_h = (src35 / 'ImageMagick/MagickCore/version.h').read_text()
assert '#define MagickLibAddendum  "-35"' in version_h and '5d1bcc55f' in version_h
generated = (src35 / 'ImageMagick/MagickCore/magick-baseconfig.h').read_text()
base34 = (prev / 'im-core/ImageMagick/MagickCore/magick-baseconfig.h').read_text()
marker = '#define MAGICKCORE_ZERO_CONFIGURATION_SUPPORT 0\n'
assert generated.endswith(marker + '\n') or generated.rstrip('\n').endswith(marker.rstrip('\n')), 'Configure output changed shape'
cut = base34.index(marker) + len(marker)
assert generated[:generated.index(marker) + len(marker)] == base34[:cut], 'option-derived baseconfig differs from -34'
with zipfile.ZipFile(selected34) as z:
    orig34 = {n: z.read(f'ImageMagick/{n}') for n in ['MagickCore/configure.c', 'MagickCore/module.c', 'MagickCore/deprecate.c',
                                                      'coders/heic.c', 'utilities/magick.c']}
for n, data in orig34.items():
    assert (src35 / 'ImageMagick' / n).read_bytes().replace(b'\r\n', b'\n') == data.replace(b'\r\n', b'\n'), f'{n} changed since -34; review patch'

# ---- 1. Prepare build tree --------------------------------------------------------
if build.exists():
    shutil.rmtree(build)  # only this script's own output directory
out.mkdir(parents=True)
for part in ['MagickCore', 'MagickWand', 'coders', 'utilities']:
    shutil.copytree(src35 / 'ImageMagick' / part, im / part)
for f in ['LICENSE', 'NOTICE']:
    shutil.copy2(src35 / 'ImageMagick' / f, im / f)
(im / 'MagickCore/magick-baseconfig.h').write_text(base34, newline='\n')
env = os.environ.copy()
env['GIT_CEILING_DIRECTORIES'] = str(build)  # not inside the IMB-C repository: git apply behaves like patch
r = subprocess.run(['git', 'apply', '-p1', '--whitespace=nowarn', str(root / 'third-party/materials/security-20261112/magick-core-isolation.patch')],
                   cwd=bsrc, env=env, capture_output=True, text=True)
assert r.returncode == 0, r.stderr
core = im / 'MagickCore'
assert 'IMB-C 2026-11-12' in (core / 'configure.c').read_text() and 'IMB-C 2026-11-12' in (core / 'module.c').read_text()

# psd: refuse files without a saved composite; single-layer output composite blended over white (same as the v1.1 build).
f = im / 'coders/psd.c'
s = f.read_text()
needle = '  if (psd_info.has_merged_image == MagickFalse)\n    {\n      Image\n        *merged;'
assert s.count(needle) == 1
s = s.replace(needle, '  if ((psd_info.has_merged_image == MagickFalse) &&\n      (image_info->number_scenes == 1) && (image_info->scene == 0))\n    ThrowReaderException(CoderError,"PSDHasNoSavedComposite");\n' + needle)
a = s.index('static MagickBooleanType CorrectPSDAlphaBlend(')
b = s.index('static inline CompressionType ConvertPSDCompression', a)
helper = s[a:b].replace('CorrectPSDAlphaBlend', 'BlendPSDCompositeWhite')
old = 'q[i]=ClampToQuantum(((double) q[i]-((1.0-gamma)*(double)\n                QuantumRange))/gamma);'
assert old in helper
s = s[:b] + helper.replace(old, 'q[i]=ClampToQuantum((double) q[i]*gamma+(1.0-gamma)*(double) QuantumRange);') + s[b:]
anchor = '      compression=image->compression;'
assert anchor in s
s = s.replace(anchor, '      if (BlendPSDCompositeWhite(image_info,image,exception) == MagickFalse)\n        ThrowWriterException(CoderError,"PSDCompositeBlendFailed");\n' + anchor, 1)
f.write_text(s + '\n/* IMB-C modification, 2026-10-27 (re-applied to 7.1.2-35 on 2026-11-13): PSD composite handling; see AVIF-REBUILD.md. */\n', newline='\n')
# heic: AVIF only, codec threads=1.
f = im / 'coders/heic.c'
s = f.read_text()
anchor = 'decode_options=heif_decoding_options_alloc();'
assert s.count(anchor) >= 1
s = s.replace(anchor, anchor + '\n  if (decode_options != (struct heif_decoding_options *) NULL)\n    decode_options->num_codec_threads=1;')
a = s.index('  entry=AcquireMagickInfo("HEIC","HEIC"')
b = s.index('  entry=AcquireMagickInfo("HEIC","AVIF"', a)
s = s[:a] + s[b:]
for o, n in [('  if (heif_have_decoder_for_format(heif_compression_AV1))\n    entry->decoder=(DecodeImageHandler *) ReadHEICImage;', '  entry->decoder=(DecodeImageHandler *) ReadHEICImage;'),
             ('  if (heif_have_encoder_for_format(heif_compression_AV1))\n    entry->encoder=(EncodeImageHandler *) WriteHEICImage;', '  entry->encoder=(EncodeImageHandler *) WriteHEICImage;')]:
    assert o in s
    s = s.replace(o, n)
assert 'AcquireMagickInfo("HEIC","HEIF"' not in s and 'AcquireMagickInfo("HEIC","AVCI"' not in s
f.write_text(s + '\n/* IMB-C modification, 2026-10-27 (re-applied to 7.1.2-35 on 2026-11-13): AVIF-only codec/thread policy; see AVIF-REBUILD.md. */\n', newline='\n')
# CLI: retain the existing runtime license notice (its base file is byte-identical between -34 and -35).
shutil.copy2(prev / 'cli-updated/ImageMagick/utilities/magick.c', im / 'utilities/magick.c')


# ---- 2. Dependency import libraries (data exports marked DATA) --------------------------------------
def exports_def(dll, name):
    head = subprocess.check_output([str(tools / 'dumpbin.exe'), '/nologo', '/headers', str(dll)], text=True, errors='replace')
    sections = []
    for block in re.split(r'SECTION HEADER #\d+', head)[1:]:
        va = int(re.search(r'([0-9A-F]+) virtual address', block).group(1), 16)
        size = int(re.search(r'([0-9A-F]+) virtual size', block).group(1), 16)
        sections.append((va, va + size, 'Execute' in block))
    text = subprocess.check_output([str(tools / 'dumpbin.exe'), '/nologo', '/exports', str(dll)], text=True, errors='replace')
    lines = []
    for rva, sym in re.findall(r'^\s+\d+\s+[0-9A-F]+\s+([0-9A-F]+)\s+(\S+)', text, re.M):
        rva = int(rva, 16)
        code = any(lo <= rva < hi and ex for lo, hi, ex in sections)
        lines.append(sym if code else f'{sym} DATA')
    path = out / f'{name}.def'
    path.write_text(f'LIBRARY {dll.name}\nEXPORTS\n' + '\n'.join(lines) + '\n')
    return path, sum(1 for x in lines if x.endswith(' DATA'))


data_exports = {}
for dep in ['bzip2', 'freetype', 'lcms', 'lqr', 'raqm', 'xml', 'zlib', 'jpeg-turbo', 'png', 'tiff', 'webp', 'lzma']:
    dll = runtime / f'CORE_RL_{dep}_.dll'
    d, n = exports_def(dll, f'CORE_RL_{dep}_')
    data_exports[dep] = n
    run([tools / 'lib.exe', '/nologo', '/machine:x64', f'/def:{d}', f'/out:{out / f"CORE_RL_{dep}_.lib"}'], f'{dep}-import')
shutil.copy2(jxl / 'out/CORE_RL_jpeg-xl_.lib', out / 'CORE_RL_jpeg-xl_.lib')
shutil.copy2(heif / 'lib/heif.lib', out / 'heif.lib')

# ---- 3. Official project settings ---------------------------------------------------------
NS = {'m': 'http://schemas.microsoft.com/developer/msbuild/2003'}


def project_sources(name):
    tree = ET.parse(src35 / f'ProjectFiles/x64/{name}/{name}.vcxproj')
    files = [n.get('Include').replace('$(SolutionDir)ImageMagick\\', '') for n in tree.findall('.//m:ClCompile[@Include]', NS)]
    return [im / f.replace('\\', '/') for f in files]


# 2026-11-13 RC：MSVC /pathmap（需搭配 /experimental:deterministic）把建置來源與專案根目錄對應成中性名稱，
# __FILE__（GetMagickModule 例外訊息）不再內嵌本機開發路徑；先列較長的來源根目錄。
PATHMAP = ['/experimental:deterministic', f'/pathmap:{bsrc}=ImageMagick-7.1.2-35', f'/pathmap:{root}=/build-user']
COMMON = ['/nologo', '/c', '/TP', '/std:c++20', '/O2', '/MD', '/openmp', '/EHsc', '/GF', '/Gy', '/source-charset:utf-8',
          '/DNDEBUG', '/D_WIN32_WINNT=0x0A00', '/D_VISUALC_', '/MP8', *PATHMAP]
SYSLIBS = ['kernel32.lib', 'user32.lib', 'gdi32.lib', 'winspool.lib', 'comdlg32.lib', 'advapi32.lib', 'shell32.lib',
           'ole32.lib', 'oleaut32.lib', 'uuid.lib', 'ws2_32.lib', 'urlmon.lib']
LINK = ['/nologo', '/MACHINE:X64', '/OPT:REF', '/OPT:ICF', '/NXCOMPAT', '/DYNAMICBASE', '/HIGHENTROPYVA', '/MAP']


def resource(kind, name):
    res = out / f'{name}.res'
    run(['rc.exe', '/nologo', f'/I{src35 / "Configure/Configs"}', f'/I{src35 / "Configure/Configs" / kind}', f'/fo{res}',
         src35 / f'Configure/Configs/{kind}/ImageMagick.rc'], f'{name}-rc')
    return res


def objdir(name):
    d = out / f'obj-{name}'
    d.mkdir(exist_ok=True)
    return d


inc34 = lambda *p: [f'/I{deps34 / x}' for x in p]
core_inc = [f'/I{src35 / "Configure/OpenCL"}', f'/I{bsrc / "ImageMagick"}',
            *inc34('bzip2', 'freetype/include', 'lcms/include', 'lqr', 'lqr/lqr', 'raqm/src', 'xml/include', 'zlib', 'glib', 'glib/glib', 'glib/include')]
o = objdir('core')
run(['cl.exe', *COMMON, '/D_MAGICKLIB_', '/D_MAGICKMOD_', *core_inc, f'/Fo{o}\\', *project_sources('CORE_MagickCore')], 'core-compile')
run(['link.exe', *LINK, '/DLL', f'/OUT:{out / "CORE_RL_MagickCore_.dll"}', f'/IMPLIB:{out / "CORE_RL_MagickCore_.lib"}',
     *sorted(o.glob('*.obj')), resource('MagickCore', 'MagickCore'),
     *[out / f'CORE_RL_{d}_.lib' for d in ['bzip2', 'freetype', 'lcms', 'lqr', 'raqm', 'xml', 'zlib']], *SYSLIBS], 'core-link')
o = objdir('wand')
run(['cl.exe', *COMMON, '/D_MAGICKLIB_', f'/I{im}', f'/Fo{o}\\', *project_sources('CORE_MagickWand')], 'wand-compile')
run(['link.exe', *LINK, '/DLL', f'/OUT:{out / "CORE_RL_MagickWand_.dll"}', f'/IMPLIB:{out / "CORE_RL_MagickWand_.lib"}',
     *sorted(o.glob('*.obj')), resource('MagickWand', 'MagickWand'), out / 'CORE_RL_MagickCore_.lib', *SYSLIBS], 'wand-link')

jxl_inc = [f'/I{jxl / "libjxl-0.13.0/lib/include"}', f'/I{jxl / "build-msvc/lib/include"}']
coders = {
    'psd': (inc34('zlib'), ['CORE_RL_zlib_.lib'], []),
    'heic': ([f'/I{heif / "include"}'], ['heif.lib'], []),
    'jpeg': (inc34('jpeg-turbo/src'), ['CORE_RL_jpeg-turbo_.lib'], []),
    'jxl': (jxl_inc, ['CORE_RL_jpeg-xl_.lib'], []),
    'meta': ([], [], []),
    # Same as the official -34 binary: MIFF keeps BZip/LZMA/Zip compression (the IMB-C core subset has no LZMA define, so add it here only).
    'miff': ([*inc34('zlib', 'bzip2'), f'/I{deps35 / "lzma/src/liblzma/api"}'], ['CORE_RL_zlib_.lib', 'CORE_RL_bzip2_.lib', 'CORE_RL_lzma_.lib'], ['/DMAGICKCORE_LZMA_DELEGATE']),
    'png': (inc34('png', 'zlib'), ['CORE_RL_png_.lib', 'CORE_RL_zlib_.lib'], []),
    'rgb': ([], [], []),
    'tiff': ([f'/I{deps35 / "tiff/libtiff"}'], ['CORE_RL_tiff_.lib', 'IM_MOD_RL_psd_.lib'], []),
    'webp': (inc34('webp/src'), ['CORE_RL_webp_.lib'], []),
}
coder_res = resource('coders', 'coders')
for name, (inc, libs, defs) in coders.items():
    o = objdir(name)
    run(['cl.exe', *COMMON, *defs, f'/I{im}', *inc, f'/Fo{o}\\', *project_sources(f'IM_MOD_{name}')], f'{name}-compile')
    run(['link.exe', *LINK, '/DLL', f'/OUT:{out / f"IM_MOD_RL_{name}_.dll"}', f'/IMPLIB:{out / f"IM_MOD_RL_{name}_.lib"}',
         *sorted(o.glob('*.obj')), coder_res, out / 'CORE_RL_MagickCore_.lib', *[out / l for l in libs], *SYSLIBS], f'{name}-link')
o = objdir('cli')
run(['cl.exe', *COMMON, f'/I{im}', f'/Fo{o}\\', im / 'utilities/magick.c'], 'cli-compile')
run(['link.exe', *LINK, '/SUBSYSTEM:CONSOLE', f'/OUT:{out / "magick.exe"}', *sorted(o.glob('*.obj')), resource('utilities', 'utilities'),
     out / 'CORE_RL_MagickCore_.lib', out / 'CORE_RL_MagickWand_.lib', *SYSLIBS], 'cli-link')

# ---- 4. Record results ------------------------------------------------------------------
products = ['CORE_RL_MagickCore_.dll', 'CORE_RL_MagickWand_.dll', 'magick.exe', *[f'IM_MOD_RL_{n}_.dll' for n in coders]]
for p in products:
    data = (out / p).read_bytes().lower()
    for text in {str(root), str(root).replace('\\', '/'), str(work)}:
        for needle in (text.lower().encode('utf-8'), text.lower().encode('utf-16-le')):
            assert needle not in data, f'{p} still embeds local path {text}'
result = {
    'imagemagick': '7.1.2-35 Q16-HDRI x64 dynamic', 'source_commit': TAG_COMMIT, 'windows_7z_sha256': ARCHIVE_SHA,
    'configure': 'Configure.Release.x64.exe /noWizard /VS2026 /x64 /dynamic /hdri /Q16',
    'magick_baseconfig_h_sha256': sha(im / 'MagickCore/magick-baseconfig.h'), 'version_h_sha256': sha(im / 'MagickCore/version.h'),
    'data_exports_marked': data_exports,
    'modified_sources_sha256': {n: sha(im / n) for n in ['MagickCore/configure.c', 'MagickCore/module.c', 'MagickCore/deprecate.c',
                                                         'coders/psd.c', 'coders/heic.c', 'utilities/magick.c']},
    'outputs': {p: sha(out / p) for p in products},
    'script_sha256': sha(Path(__file__)),
}
(out / 'build-result.json').write_text(json.dumps(result, indent=2, ensure_ascii=False), encoding='utf-8')
print(json.dumps(result['outputs'], indent=2))
