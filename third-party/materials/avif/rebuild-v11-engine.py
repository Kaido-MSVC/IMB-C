from pathlib import Path
import os,sys,subprocess,zipfile,tarfile
root=Path(__file__).resolve().parents[1];os.chdir(root);work=root/'work';work.mkdir(exist_ok=True)
with zipfile.ZipFile(root/'third-party/materials/avif/libheif-1.23.6.zip') as z:z.extractall(work/'native-src')
with tarfile.open(root/'third-party/materials/avif/libaom-3.15.2.tar.gz') as z:z.extractall(work/'native-src',filter='data')
with zipfile.ZipFile(root/'third-party/materials/ImageMagick-7.1.2-34-selected-corresponding-source.zip') as z:
 for n in z.namelist():
  if n.startswith('ImageMagick/') and not n.endswith('/'):z.extract(n,work/'im-src')
with zipfile.ZipFile(root/'third-party/cli-notice-source.zip') as z:z.extractall(work/'cli-src')
for name in ['build-v11-native.py','build-v11-modules.py']:subprocess.run([sys.executable,'-X','utf8',str(root/'scripts'/name)],check=True)
print('New outputs: work/heif-install/bin/heif.dll and work/rebuilt-modules. Replace only your private candidate after verification.')
