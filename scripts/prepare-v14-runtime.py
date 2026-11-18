"""Prepare a development runtime without changing any v1.3 release files."""
from pathlib import Path
import hashlib
import importlib.util
import json
import shutil
import sys


def prepare(source, destination, root):
    if source == destination or source in destination.parents:
        raise RuntimeError("Use an isolated destination outside the source runtime")
    spec = importlib.util.spec_from_file_location("v13", root / "scripts/prepare-v13-runtime.py")
    v13 = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(v13)
    baseline = json.loads((root / "RUNTIME-MANIFEST-v1.3.json").read_text())
    v13.verify(source, baseline)
    if destination.exists():
        raise RuntimeError("Destination already exists; refusing to overwrite")
    destination.mkdir(parents=True)
    for rel in baseline:
        target = destination / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source / rel, target)
    shutil.copyfile(root / "assets/policy-v14.xml", destination / "ImageMagick/policy.xml")
    shutil.copytree(source / "licenses", destination / "licenses", dirs_exist_ok=True)
    candidate = json.loads((root / "RUNTIME-MANIFEST-v1.4.json").read_text())
    v13.verify(destination, candidate)
    v13.verify(source, baseline)
    return hashlib.sha256((destination / "ImageMagick/policy.xml").read_bytes()).hexdigest()


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    if len(sys.argv) != 3:
        raise SystemExit("Usage: prepare-v14-runtime.py <v1.3.3 runtime root> <new candidate root>")
    print("Verified isolated v1.4 runtime; policy SHA-256:", prepare(
        Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve(), root))
