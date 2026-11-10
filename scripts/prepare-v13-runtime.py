"""Validate all v1.3 runtime files before copying; never repair from partial input."""
from pathlib import Path
import hashlib
import json
import shutil
import sys


def verify(source, manifest):
    for rel, digest in manifest.items():
        path = source / rel
        if not path.is_file():
            raise RuntimeError("Missing runtime: " + rel)
        with path.open("rb") as stream:
            actual = hashlib.file_digest(stream, "sha256").hexdigest()
        if actual != digest:
            raise RuntimeError("Mismatched runtime: " + rel)


def prepare(source, destination, manifest):
    verify(source, manifest)
    if source != destination:
        if not (source / "licenses").is_dir():
            raise RuntimeError("Missing runtime licenses directory")
        for rel in manifest:
            target = destination / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / rel, target)
        shutil.copytree(source / "licenses", destination / "licenses", dirs_exist_ok=True)
    verify(destination, manifest)


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    destination = root / "target/release"
    source = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else destination
    manifest = json.loads((root / "RUNTIME-MANIFEST-v1.3.json").read_text())
    prepare(source, destination, manifest)
    print("Verified v1.3 private runtime; no global installation modified.")
