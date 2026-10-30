from pathlib import Path
import os,subprocess,json,re
p=Path('work').resolve(); cm=Path(r'C:\Program Files\Microsoft Visual Studio\18\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe')
commands=[]
def run(args,label):
 commands.append(list(map(str,args))); (p/'native-commands.json').write_text(json.dumps(commands,indent=2))
 with (p/(label+'.log')).open('wb') as f:r=subprocess.run(list(map(str,args)),stdout=f,stderr=subprocess.STDOUT)
 print(label,r.returncode,flush=True)
 if r.returncode:print((p/(label+'.log')).read_text(errors='replace')[-6000:]);raise SystemExit(r.returncode)
common=['-G','Visual Studio 18 2026','-A','x64','-DCMAKE_POLICY_VERSION_MINIMUM=3.5','-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL']
run([cm,'-S',p/'native-src/libaom-3.15.2','-B',p/'aom-build',*common,'-DBUILD_SHARED_LIBS=OFF','-DAOM_TARGET_CPU=generic','-DENABLE_TESTS=OFF','-DENABLE_EXAMPLES=OFF','-DENABLE_TOOLS=OFF','-DENABLE_DOCS=OFF','-DCONFIG_AV1_DECODER=1','-DCONFIG_AV1_ENCODER=1'],'aom-configure')
run([cm,'--build',p/'aom-build','--config','Release','--parallel','4'],'aom-build')
heif=p/'native-src/libheif-1.23.6'
# Fixed one-thread AOM encoder default, saved patched source will accompany this build.
f=heif/'libheif/plugins/encoder_aom.cc';s=f.read_text();s=s.replace('p->integer.default_value = threads;','p->integer.default_value = 1;');f.write_text(s+'\n/* IMB-C modification, 2026-10-27: AVIF-only codec/thread policy, PSD composite handling or updated runtime notices; see AVIF-REBUILD.md. */\n')
opts=re.findall(r'^plugin_option\((\w+)',(heif/'CMakeLists.txt').read_text(),re.M)
flags=[f'-DWITH_{x}='+('ON' if x in ['AOM_DECODER','AOM_ENCODER'] else 'OFF') for x in opts]
flags += [f'-DWITH_{x}_PLUGIN=OFF' for x in opts]
flags += ['-DWITH_UNCOMPRESSED_CODEC=OFF','-DWITH_WEBCODECS=OFF','-DENABLE_PLUGIN_LOADING=OFF','-DENABLE_PARALLEL_TILE_DECODING=OFF','-DWITH_LIBSHARPYUV=OFF','-DWITH_HEADER_COMPRESSION=OFF','-DWITH_EXAMPLES=OFF','-DWITH_GDK_PIXBUF=OFF','-DBUILD_TESTING=OFF','-DBUILD_DOCUMENTATION=OFF','-DBUILD_SHARED_LIBS=ON','-DCMAKE_SHARED_LINKER_FLAGS=/MAP',f'-DAOM_INCLUDE_DIR={p}/native-src/libaom-3.15.2',f'-DAOM_LIBRARY={p}/aom-build/Release/aom.lib',f'-DCMAKE_INSTALL_PREFIX={p}/heif-install']
run([cm,'-S',heif,'-B',p/'heif-build',*common,*flags],'heif-configure')
run([cm,'--build',p/'heif-build','--config','Release','--parallel','4'],'heif-build')
run([cm,'--install',p/'heif-build','--config','Release'],'heif-install')
