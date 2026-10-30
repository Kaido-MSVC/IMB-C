from pathlib import Path
import json,hashlib,shutil,sys
root=Path(__file__).resolve().parents[1];dst=root/'target/release'
source=Path(sys.argv[1]).resolve() if len(sys.argv)>1 else dst
manifest=json.loads((root/'RUNTIME-MANIFEST-v1.3.json').read_text())
for rel,digest in manifest.items():
 p=source/rel
 assert p.is_file() and hashlib.file_digest(p.open('rb'),'sha256').hexdigest()==digest,'Missing/mismatched runtime: '+rel
 if source!=dst:
  target=dst/rel;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,target)
if source!=dst:shutil.copytree(source/'licenses',dst/'licenses',dirs_exist_ok=True)
print('Verified v1.3 private runtime; no global installation modified.')
