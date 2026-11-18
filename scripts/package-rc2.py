"""由已驗證輸入建立全新的本機 RC 封包，不沿用歷史封包。"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
ROOT_FILES = {
    ".gitignore", "Cargo.toml", "Cargo.lock", "build.rs", "LICENSE", "README.md",
    "AVIF-REBUILD.md", "FREETYPE-NOTICE.md", "LGPL-REPLACEMENT.md",
    "RUNTIME-MANIFEST-v1.3.json", "RUNTIME-MANIFEST-v1.4.json",
    "THIRD-PARTY-NOTICES.md", "THIRD-PARTY-RUNTIME.txt",
    "THIRD-PARTY-LICENSES-v1.2.0.md", "THIRD-PARTY-LICENSES-v1.3.0.md",
}
PREFIXES = ("src/", "assets/", "tests/", "scripts/", "third-party/", "docs/")
FORBIDDEN = {"iat.exe", "iat-worker.exe"}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def files(root):
    result = []
    for path in root.rglob("*"):
        if path.is_symlink() or path.is_junction():
            raise RuntimeError("Refusing reparse point: " + str(path))
        if path.is_file():
            if path.name.lower() in FORBIDDEN or path.suffix.lower() == ".pdb":
                raise RuntimeError("Private tool or PDB is not a package input")
            result.append(path)
    return sorted(result)


def write_archive(path, inputs):
    expected = {name: digest(source) for source, name in inputs}
    if len(expected) != len(inputs):
        raise RuntimeError("Duplicate archive paths")
    with zipfile.ZipFile(path, "x", zipfile.ZIP_DEFLATED, compresslevel=6, strict_timestamps=False) as archive:
        for source, name in inputs:
            archive.write(source, name)
    with zipfile.ZipFile(path) as archive:
        if archive.testzip() is not None or set(archive.namelist()) != set(expected):
            raise RuntimeError("Archive content verification failed")
        for name, sha in expected.items():
            with archive.open(name) as stream:
                if hashlib.file_digest(stream, "sha256").hexdigest() != sha:
                    raise RuntimeError("Archive hash mismatch: " + name)
    return {"files": len(inputs), "bytes": path.stat().st_size, "sha256": digest(path)}


def package(stage, destination, gates):
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    if version != "1.4.0-rc.2":
        raise RuntimeError("This packager is scoped to the verified RC2 version")
    if gates.get("version") != version or not gates.get("ready_for_local_rc"):
        raise RuntimeError("Final local RC gates are not satisfied")
    if not gates.get("privacy_verified") or not gates.get("sources_preserved"):
        raise RuntimeError("Privacy and source-preservation gates are required")
    stage = stage.resolve()
    destination = destination.resolve()
    if stage == destination or stage in destination.parents or destination in stage.parents:
        raise RuntimeError("Keep archives and staging independent")
    if destination.exists():
        raise RuntimeError("Archive destination must be new")
    for required in ["imb-c.exe", "imb-c-cli.exe", "DEPENDENCIES-v1.4.json", "docs/v1.4-rc2-acceptance.md", "docs/v1.4-rc2-known-issues.md"]:
        if not (stage / required).is_file():
            raise RuntimeError("Missing verified input: " + required)
    runtime = json.loads((ROOT / "RUNTIME-MANIFEST-v1.4.json").read_text())
    for name, sha in runtime.items():
        if digest(stage / name) != sha:
            raise RuntimeError("Runtime mismatch: " + name)
    if json.loads((stage / "DEPENDENCIES-v1.4.json").read_text())["application"] != version:
        raise RuntimeError("Dependency inventory version mismatch")
    source_paths = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard"], cwd=ROOT, text=True, encoding="utf-8").splitlines()
    source_paths = sorted(name for name in source_paths if name in ROOT_FILES or name.startswith(PREFIXES))
    for name in ["src/validation_queue.rs", "src/iat.rs", "src/cli.rs", "scripts/package-rc2.py"]:
        if name not in source_paths:
            raise RuntimeError("Required source is not in the public allowlist: " + name)
    source_inputs = []
    for name in source_paths:
        path = ROOT / name
        if path.is_symlink() or path.is_junction() or not path.is_file() or path.suffix.lower() in {".pdb", ".exe", ".dll", ".pyc"}:
            raise RuntimeError("Unexpected application source entry: " + name)
        source_inputs.append((path, "source/" + name))
    for path in files(stage / "licenses"):
        source_inputs.append((path, "native-materials/licenses/" + path.relative_to(stage / "licenses").as_posix()))
    destination.mkdir()
    payload = [path for path in files(stage) if path != stage / "SHA256SUMS.txt"]
    manifest = "".join(digest(path) + "  " + path.relative_to(stage).as_posix() + "\n" for path in payload)
    checksum = stage / "SHA256SUMS.txt"
    if checksum.exists():
        raise RuntimeError("Staging manifest already exists; do not replace verified inputs")
    checksum.write_text(manifest, encoding="utf-8", newline="\n")
    portable = destination / f"IMB-C-v{version}-Portable-x64.zip"
    source = destination / f"IMB-C-v{version}-Source.zip"
    portable_result = write_archive(portable, [(path, f"IMB-C-v{version}/" + path.relative_to(stage).as_posix()) for path in files(stage)])
    source_result = write_archive(source, source_inputs)
    for name in ["v1.4-rc2-acceptance.md", "v1.4-rc2-known-issues.md", "v1.4-rc2-review.md", "v1.4-rc1-to-rc2.md"]:
        shutil.copy2(stage / "docs" / name, destination / name)
    checks = [portable, source] + sorted(destination.glob("*.md"))
    (destination / "SHA256SUMS.txt").write_text("".join(digest(path) + "  " + path.name + "\n" for path in checks), encoding="utf-8", newline="\n")
    return {"version": version, "portable": portable_result, "source": source_result, "application_source_files": len(source_paths)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("stage", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("gates", type=Path)
    args = parser.parse_args()
    print(json.dumps(package(args.stage, args.destination, json.loads(args.gates.read_text(encoding="utf-8"))), indent=2))
