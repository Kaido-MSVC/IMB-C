from pathlib import Path
import os,subprocess,json,re,shutil
root=Path(__file__).resolve().parents[1];p=Path('work').resolve();src=p/'im-src/ImageMagick';headers=p/'cli-src/ImageMagick';out=p/'rebuilt-modules';out.mkdir(exist_ok=True)
# Reuse the exact configured Q16-HDRI header set used by the baseline CLI.
for f in (headers/'MagickCore').glob('*.h'):shutil.copy2(f,src/'MagickCore'/f.name)
import zipfile
with zipfile.ZipFile(root/'third-party/materials/ImageMagick-7.1.2-34-selected-corresponding-source.zip') as z:
 for n in ['heic','psd']:(src/f'coders/{n}.c').write_bytes(z.read(f'ImageMagick/coders/{n}.c'))
# PSD without a saved composite must fail instead of falling back to layer rendering.
f=src/'coders/psd.c';s=f.read_text();needle='  if (psd_info.has_merged_image == MagickFalse)\n    {\n      Image\n        *merged;';assert needle in s
s=s.replace(needle,'  if ((psd_info.has_merged_image == MagickFalse) &&\n      (image_info->number_scenes == 1) && (image_info->scene == 0))\n    ThrowReaderException(CoderError,"PSDHasNoSavedComposite");\n'+needle) 
# Single-layer output stores unassociated layer RGB and white-blended composite RGB.
a=s.index('static MagickBooleanType CorrectPSDAlphaBlend(');b=s.index('static inline CompressionType ConvertPSDCompression',a)
helper=s[a:b].replace('CorrectPSDAlphaBlend','BlendPSDCompositeWhite')
old='q[i]=ClampToQuantum(((double) q[i]-((1.0-gamma)*(double)\n                QuantumRange))/gamma);'
new='q[i]=ClampToQuantum((double) q[i]*gamma+(1.0-gamma)*(double) QuantumRange);'
assert old in helper;helper=helper.replace(old,new)
s=s[:b]+helper+s[b:]
s=s.replace('      compression=image->compression;', '      if (BlendPSDCompositeWhite(image_info,image,exception) == MagickFalse)\n        ThrowWriterException(CoderError,"PSDCompositeBlendFailed");\n      compression=image->compression;',1)
f.write_text(s+'\n/* IMB-C modification, 2026-10-27: AVIF-only codec/thread policy, PSD composite handling or updated runtime notices; see AVIF-REBUILD.md. */\n')
# Both HEIC decode paths pass an explicit codec-thread budget.
f=src/'coders/heic.c';s=f.read_text();s=s.replace('decode_options=heif_decoding_options_alloc();','decode_options=heif_decoding_options_alloc();\n  if (decode_options != (struct heif_decoding_options *) NULL)\n    decode_options->num_codec_threads=1;');# Product coder exposes AVIF only; no HEIC/HEIF/AVCI registration.
a=s.index('  entry=AcquireMagickInfo("HEIC","HEIC"');b=s.index('  entry=AcquireMagickInfo("HEIC","AVIF"',a);s=s[:a]+s[b:]
s=s.replace('  if (heif_have_decoder_for_format(heif_compression_AV1))\n    entry->decoder=(DecodeImageHandler *) ReadHEICImage;', '  entry->decoder=(DecodeImageHandler *) ReadHEICImage;').replace('  if (heif_have_encoder_for_format(heif_compression_AV1))\n    entry->encoder=(EncodeImageHandler *) WriteHEICImage;', '  entry->encoder=(EncodeImageHandler *) WriteHEICImage;')
f.write_text(s+'\n/* IMB-C modification, 2026-10-27: AVIF-only codec/thread policy, PSD composite handling or updated runtime notices; see AVIF-REBUILD.md. */\n')
msvc=Path(r'C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.51.36231');sdk=Path(r'C:\Program Files (x86)\Windows Kits\10');tools=msvc/'bin/Hostx64/x64';env=os.environ.copy();env['INCLUDE']=';'.join(map(str,[msvc/'include',sdk/'Include/10.0.28000.0/ucrt',sdk/'Include/10.0.28000.0/shared',sdk/'Include/10.0.28000.0/um']));env['LIB']=';'.join(map(str,[msvc/'lib/x64',sdk/'Lib/10.0.28000.0/ucrt/x64',sdk/'Lib/10.0.28000.0/um/x64',out,p/'heif-install/lib']))
commands=[]
def run(args,label):
 commands.append(list(map(str,args)));(out/'commands.json').write_text(json.dumps(commands,indent=2));r=subprocess.run(list(map(str,args)),env=env,cwd=out,capture_output=True);(out/(label+'.log')).write_bytes(r.stdout+r.stderr);print(label,r.returncode,flush=True)
 if r.returncode:print((r.stdout+r.stderr).decode(errors='replace')[-7000:]);raise SystemExit(r.returncode)
for name in ['MagickCore','MagickWand','zlib']:
 deff=out/f'CORE_RL_{name}_.def';shutil.copy2(root/f'third-party/materials/avif/CORE_RL_{name}_.def',deff)
 run([tools/'lib.exe','/nologo','/machine:x64',f'/def:{deff}',f'/out:{out}/CORE_RL_{name}_.lib'],name+'-import')
# Source ZIP also contains the matching zlib header; extract from original ZIP if needed.
import zipfile
with zipfile.ZipFile(root/'third-party/materials/ImageMagick-7.1.2-34-selected-corresponding-source.zip') as z:
 for n in z.namelist():
  if n.startswith('Dependencies/Dependencies/zlib/') and n.endswith('.h'):z.extract(n,p/'zlib-src')
for name in ['heic','psd']:
 extra=['/DMAGICKCORE_HEIC_DELEGATE',f'/I{p}/heif-install/include'] if name=='heic' else [f'/I{p}/zlib-src/Dependencies/Dependencies/zlib']
 run([tools/'cl.exe','/nologo','/utf-8','/TP','/O2','/MD','/EHsc','/LD','/DNDEBUG','/D_VISUALC_','/D_WIN32_WINNT=0x0A00',f'/I{src}',*extra,src/f'coders/{name}.c',f'/Fo{out}/{name}.obj',f'/Fe{out}/IM_MOD_RL_{name}_.dll','/link','/MAP',('heif.lib' if name=='heic' else 'CORE_RL_zlib_.lib'),'CORE_RL_MagickCore_.lib','kernel32.lib','user32.lib','advapi32.lib'],name+'-build')

# Runtime notice must also cover the newly loaded LGPL libheif DLL.
f=headers/'utilities/magick.c';s=f.read_text()
needle='    "Full authors, corresponding source and replacement instructions:'
if 'libheif 1.23.6' not in s:
 s=s.replace(needle,'    "libheif 1.23.6: Copyright 2017 Dirk Farin and source-file authors. LGPL-3.0-or-later.\\n"\n    "License/source/build and replacement: ../licenses/avif and ../AVIF-REBUILD.md. No warranty.\\n"\n    "libaom 3.15.2: Copyright 2016 Alliance for Open Media. BSD-2-Clause and PATENTS: ../licenses/avif.\\n"\n'+needle)
f.write_text(s+'\n/* IMB-C modification, 2026-10-27: AVIF-only codec/thread policy, PSD composite handling or updated runtime notices; see AVIF-REBUILD.md. */\n')
run([tools/'cl.exe','/nologo','/utf-8','/TP','/O2','/MD','/EHsc','/DNDEBUG','/D_WIN32_WINNT=0x0A00',f'/I{headers}',f,f'/Fo{out}/magick.obj',f'/Fe{out}/magick.exe','/link','/SUBSYSTEM:CONSOLE','CORE_RL_MagickCore_.lib','CORE_RL_MagickWand_.lib','kernel32.lib','user32.lib','advapi32.lib'],'cli-build')
