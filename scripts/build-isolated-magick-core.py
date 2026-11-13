"""Rebuild only the existing 7.1.2-34 Q16-HDRI core, using retained headers/dependencies.

Input: work/security-followup-20261112/im-core (source prepared and patched there).
Output stays in work; never replaces runtime automatically.
"""
from pathlib import Path
import os
import re
import subprocess
import json
import hashlib

root = Path(__file__).resolve().parents[1]
work = root / 'work/security-followup-20261112'
source = work / 'im-core/ImageMagick'
deps = work / 'im-core/Dependencies/Dependencies'
out = work / 'isolated-core'
out.mkdir(exist_ok=True)
msvc = Path(r'C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.52.37025')
tools = msvc / 'bin/Hostx64/x64'
sdk = Path(r'C:\Program Files (x86)\Windows Kits\10')
env = os.environ.copy()
env['INCLUDE'] = ';'.join(map(str, [msvc/'include', sdk/'Include/10.0.28000.0/ucrt', sdk/'Include/10.0.28000.0/shared', sdk/'Include/10.0.28000.0/um']))
env['LIB'] = ';'.join(map(str, [msvc/'lib/x64', sdk/'Lib/10.0.28000.0/ucrt/x64', sdk/'Lib/10.0.28000.0/um/x64', out]))
commands = []

def run(args, label):
    args = list(map(str, args))
    commands.append(args)
    (out/'commands.json').write_text(json.dumps(commands, indent=2))
    with (out/(label+'.log')).open('wb') as log:
        result = subprocess.run(args, cwd=out, env=env, stdout=log, stderr=subprocess.STDOUT)
    print(label, result.returncode, flush=True)
    if result.returncode:
        print((out/(label+'.log')).read_text(errors='replace')[-8000:])
        raise SystemExit(result.returncode)

libraries = []
for name in ['bzip2', 'freetype', 'lcms', 'lqr', 'raqm', 'xml', 'zlib']:
    dll = root / f'target/release/ImageMagick/CORE_RL_{name}_.dll'
    exports = subprocess.check_output([str(tools/'dumpbin.exe'), '/exports', str(dll)], text=True)
    names = re.findall(r'^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]+\s+(\S+)', exports, re.M)
    definition = out/f'CORE_RL_{name}_.def'
    definition.write_text(f'LIBRARY CORE_RL_{name}_.dll\nEXPORTS\n'+'\n'.join(names)+'\n')
    library = out/f'CORE_RL_{name}_.lib'
    run([tools/'lib.exe', '/nologo', '/machine:x64', f'/def:{definition}', f'/out:{library}'], name+'-import')
    libraries.append(library)

includes = [work / "im-core/Configure/OpenCL", source, deps/'bzip2', deps/'freetype/include', deps/'lcms/include', deps/'lqr', deps/'lqr/lqr', deps/'raqm/src', deps/'xml/include', deps/'zlib', deps/'glib', deps/'glib/glib', deps/'glib/include']
inputs = sorted([*(source/'MagickCore').glob('*.c'), *(source/'MagickCore').glob('*.h'), *source.joinpath('coders').glob('*.h')])
signature = hashlib.sha256(b'c++17-MD-openmp-isolation-v1'+b''.join(p.read_bytes() for p in inputs)).hexdigest()
stamp = out/'build-input-sha256.txt'
rebuild = not stamp.exists() or stamp.read_text() != signature
sources = [p for p in (source/'MagickCore').glob('*.c') if p.name != 'DllMain.c' and (rebuild or not (out/(p.stem+'.obj')).exists() or p.stat().st_mtime > (out/(p.stem+'.obj')).stat().st_mtime)]
if sources: run([tools/'cl.exe', '/nologo', '/utf-8', '/TP', '/EHsc', '/std:c++17', '/O2', '/MD', '/openmp', '/MP4', '/c', '/DNDEBUG', '/D_VISUALC_', '/D_MAGICKLIB_', '/D_MAGICKMOD_', '/D_WIN32_WINNT=0x0A00', *[f'/I{p}' for p in includes], *sources], 'core-compile')
run([tools/'link.exe', '/nologo', '/DLL', '/MACHINE:X64', '/MAP', f'/OUT:{out}/CORE_RL_MagickCore_.dll', *[out/(p.stem+'.obj') for p in (source/'MagickCore').glob('*.c') if p.name != 'DllMain.c'], *libraries, 'kernel32.lib', 'user32.lib', 'gdi32.lib', 'advapi32.lib', 'ws2_32.lib'], 'core-link')
stamp.write_text(signature)
wand = out/'CORE_RL_MagickWand_.def'
wand.write_bytes((root/'third-party/materials/avif/CORE_RL_MagickWand_.def').read_bytes())
run([tools/'lib.exe', '/nologo', '/machine:x64', f'/def:{wand}', f'/out:{out}/CORE_RL_MagickWand_.lib'], 'wand-import')
run([tools/'cl.exe', '/nologo', '/utf-8', '/TP', '/O2', '/MD', '/EHsc', '/DNDEBUG', '/D_WIN32_WINNT=0x0A00', f'/I{source}', work/'cli-updated/ImageMagick/utilities/magick.c', f'/Fo{out}/magick-cli.obj', f'/Fe{out}/magick.exe', '/link', '/SUBSYSTEM:CONSOLE', 'CORE_RL_MagickCore_.lib', 'CORE_RL_MagickWand_.lib', 'kernel32.lib', 'user32.lib', 'advapi32.lib'], 'cli-build')

