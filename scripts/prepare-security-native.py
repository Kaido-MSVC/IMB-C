"""Prepare retained sources for the 2026-11-12 native rebuild; no runtime replacement.

Uses local source/license archives only. Prepares the same changes documented by
magick-core-isolation.patch; then invoke build-isolated-magick-core.py.
"""
from pathlib import Path
import hashlib
import zipfile
import tarfile

root = Path(__file__).resolve().parents[1]
work = root/'work/security-followup-20261112'
work.mkdir(exist_ok=True)
licenses = root/'target/release/licenses'
heif_archive = root/'third-party/materials/avif/libheif-1.23.7.zip'
assert hashlib.sha256(heif_archive.read_bytes()).hexdigest() == 'b41400d7e461a15b82b505681e762ee606c21a08c69e5d807ed2fe7af063d47e'
assert hashlib.sha256((licenses/'avif/libaom-3.15.2.tar.gz').read_bytes()).hexdigest() == '67bb54b245f33ed98600e08269e6139986e48114e14042474ddd8885801dfddc'
with zipfile.ZipFile(heif_archive) as z:
    z.extractall(work/'native-src')
with tarfile.open(licenses/'avif/libaom-3.15.2.tar.gz') as t:
    t.extractall(work/'native-src', filter='data')
native = (root/'third-party/materials/avif/build-v11-native.py').read_text()
native = native.replace("Path('work').resolve()", "Path('work/security-followup-20261112').resolve()").replace('libheif-1.23.6', 'libheif-1.23.7')
(work/'build-native.py').write_text(native)

source = work/'im-core'
with zipfile.ZipFile(licenses/'ImageMagick-7.1.2-34-selected-corresponding-source.zip') as z:
    for name in z.namelist():
        if name.startswith(('ImageMagick/', 'Configure/OpenCL/')) or (name.startswith('Dependencies/') and name.endswith(('.h', '.hpp', '.inl'))):
            z.extract(name, source)
for name in ['configure.c', 'module.c', 'deprecate.c']:
    path = source/'ImageMagick/MagickCore'/name
    path.write_text(path.read_text(), newline='\n')
# 固定版本來源的兩個搜尋函式只保留 DLL 相對路徑。
def replace_body(path, marker, body, after=0):
    text = path.read_text()
    start = text.index('{', text.index(marker, after))
    end = text.index('\n}\n', start)+2
    path.write_text(text[:start]+body+text[end:], newline='\n')

core = source/'ImageMagick/MagickCore'
replace_body(core/'configure.c', 'MagickExport LinkedListInfo *GetConfigurePaths(', r'''{
  char path[MagickPathExtent];
  LinkedListInfo *paths=NewLinkedList(0);
  /* IMB-C 2026-11-12: only the loaded core DLL supplies configuration. */
  if (NTGetModulePath("CORE_RL_MagickCore_.dll",path) == MagickFalse)
    { (void) ThrowMagickException(exception,GetMagickModule(),ConfigureError,
        "UnableToGetModulePath","`%s'",filename); return(paths); }
  (void) ConcatenateMagickString(path,DirectorySeparator,MagickPathExtent);
  (void) AppendValueToLinkedList(paths,ConstantString(path));
  return(paths);
}''')
module = core/'module.c'
replace_body(module, 'static MagickBooleanType GetMagickModulePath(', r'''{
  /* IMB-C 2026-11-12: ignore registry, environment and user module paths. */
  if ((strchr(filename,'/') != (char *) NULL) ||
      (strchr(filename,'\\') != (char *) NULL) ||
      (strchr(filename,':') != (char *) NULL) ||
      (NTGetModulePath("CORE_RL_MagickCore_.dll",path) == MagickFalse))
    return(MagickFalse);
  (void) ConcatenateMagickString(path,module_type == MagickImageCoderModule ?
    "\\modules\\coders\\" : "\\modules\\filters\\",MagickPathExtent);
  (void) ConcatenateMagickString(path,filename,MagickPathExtent);
  if (IsPathAccessible(path) == MagickFalse) return(MagickFalse);
  return(MagickTrue);
}''', module.read_text().index('%  GetMagickModulePath()'))
deprecate = core/'deprecate.c'
deprecate.write_text(deprecate.read_text().replace('#if !defined(MAGICKCORE_EXCLUDE_DEPRECATED)',
    '/* IMB-C: retain the four legacy C++ exports present in the baseline DLL. */\n#undef MAGICKCORE_EXCLUDE_DEPRECATED\n#if !defined(MAGICKCORE_EXCLUDE_DEPRECATED)', 1), newline='\n')
with zipfile.ZipFile(licenses/'cli-notice-source.zip') as z:
    for name in z.namelist():
        if name.startswith('ImageMagick/MagickCore/') and name.endswith('.h'):
            z.extract(name, source)
header = source/'ImageMagick/MagickCore/magick-baseconfig.h'
header.write_text(header.read_text()+'\n/* IMB-C: AVIF/TIFF/JXL module aliases match the packaged runtime. */\n#define MAGICKCORE_HEIC_DELEGATE\n#define MAGICKCORE_TIFF_DELEGATE\n#define MAGICKCORE_JXL_DELEGATE\n')
with zipfile.ZipFile(licenses/'avif/modified-source.zip') as z:
    cli = work/'cli-updated/ImageMagick/utilities/magick.c'
    cli.parent.mkdir(parents=True, exist_ok=True)
    cli.write_text(z.read('ImageMagick/utilities/magick.c').decode().replace('\r\n', '\n').replace('libheif 1.23.6', 'libheif 1.23.7'), newline='\n')
print('Prepared exact existing-version sources; no runtime file replaced.')
