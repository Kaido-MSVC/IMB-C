"""建立 RC2 Runtime 更新版的本機 Portable ZIP 與 Source ZIP（MSI 由 installer/wix7-v1.4.0-rc.2 另行建置）。

輸入均須是新的、已驗證的目錄；不覆寫任何既有封包或證據。

  python -X utf8 scripts/package-rc2-runtime.py --runtime <update-rc2-vc-runtime.py 的輸出>
      --exe-dir <含 Release imb-c.exe、imb-c-cli.exe 的目錄> --jxl-materials <native-materials/jxl-rc2 內容>
      --dependencies <DEPENDENCIES-v1.4.json> --dumpbin <dumpbin.exe> --dest <新目錄>
"""
import argparse
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = "1.4.0-rc.2"
REPO_FILES = (
    "LICENSE", "README.md", "THIRD-PARTY-NOTICES.md", "THIRD-PARTY-RUNTIME.txt", "RUNTIME-MANIFEST-v1.4.json",
    "RUNTIME-RECIPIENT-TERMS.md", "RUNTIME-HANDOFF.md", "RELEASE-NOTES-1.4.0-rc.2.md", "AVIF-REBUILD.md",
    "FREETYPE-NOTICE.md", "LGPL-REPLACEMENT.md", "THIRD-PARTY-LICENSES-v1.2.0.md", "THIRD-PARTY-LICENSES-v1.3.0.md",
)
DOCS = (
    "v1.4-rc2-build.md", "v1.4-rc2-known-issues.md", "v1.4-cli-json-contract.md", "v1.4-rc2-appcontainer.md",
    "v1.4-rc2-runtime-delivery.md", "v1.4-rc2-license-review.md", "v1.4-rc2-jxl-source.md",
    "v1.4-rc2-runtime-acceptance.md",
)
LICENSE_EXTRAS = {
    "licenses/microsoft/RUNTIME-VC-UPDATE-1.4.0-rc.2.json": "third-party/materials/microsoft/RUNTIME-VC-UPDATE-1.4.0-rc.2.json",
    "licenses/security-20261113/RC2-SOURCES.json": "third-party/materials/security-20261113/RC2-SOURCES.json",
}
EXTRA_SOURCE_FILES = {".cargo/config.toml", "RUNTIME-RECIPIENT-TERMS.md", "RUNTIME-HANDOFF.md", "RELEASE-NOTES-1.4.0-rc.2.md"}
EXTRA_SOURCE_PREFIXES = ("installer/wix7-v1.4.0-rc.2/",)
CRT_IMPORTS = ("vcruntime", "msvcp", "vcomp", "api-ms-win-crt", "ucrtbase", "concrt")


def load_packager():
    spec = importlib.util.spec_from_file_location("imbc_packager", ROOT / "scripts/package-rc2.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def inventory(root):
    result = {}
    for path in sorted(root.rglob("*")):
        if path.lstat().st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT:
            raise RuntimeError(f"Reparse point: {path}")
        if path.is_file():
            result[path.relative_to(root).as_posix()] = sha(path)
    return result


def copy_new(source, target):
    if target.exists():
        raise RuntimeError(f"Refusing to overwrite {target}")
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)


def imports(dumpbin, exe):
    out = subprocess.run([str(dumpbin), "/nologo", "/dependents", str(exe)], check=True,
                         capture_output=True, text=True, encoding="utf-8", errors="replace").stdout
    return sorted({line.strip().lower() for line in out.splitlines() if line.strip().lower().endswith(".dll")})


def cli_has_no_terms_gate(cli):
    """The Runtime terms ship as a document only; the CLI has no consent subcommand."""
    with tempfile.TemporaryDirectory() as config:
        env = dict(os.environ, IMBC_CONFIG_DIR=config)
        result = subprocess.run([str(cli), "runtime-terms", "status"], capture_output=True,
                                text=True, encoding="utf-8", env=env, timeout=60)
        help_text = subprocess.run([str(cli), "--help"], check=True, capture_output=True,
                                   text=True, encoding="utf-8", env=env, timeout=60).stdout
    return result.returncode == 2 and "runtime-terms" not in help_text


def scan_privacy(inputs, extra_roots):
    home = Path(os.environ["USERPROFILE"])
    # 其他私人位置由執行者以 --private-root 提供，不寫入公開腳本。
    paths = [ROOT, home, ROOT.parent / "Image Analysis Tool", *extra_roots]
    needles = sorted({form.lower() for path in paths for form in (str(path), path.as_posix())})
    patterns = [(needle, needle.encode(enc)) for needle in needles for enc in ("utf-8", "utf-16le")]
    hits, count = [], 0

    def scan(data, name, depth=0):
        nonlocal count
        count += 1
        base = Path(name.split("!")[-1])
        if base.name.lower() in {"iat.exe", "iat-worker.exe"} or base.suffix.lower() in {".pdb", ".wixpdb", ".pfx", ".p12", ".pyc"}:
            hits.append([name, "excluded file type or private executable"])
        lower = data.lower()
        hits.extend([name, label] for label, encoded in patterns if encoded in lower)
        if depth < 2 and name.lower().endswith((".zip", ".docx")):
            with zipfile.ZipFile(io.BytesIO(data)) as archive:
                for info in archive.infolist():
                    if not info.is_dir():
                        scan(archive.read(info), f"{name}!{info.filename}", depth + 1)

    for path, name in inputs:
        scan(path.read_bytes(), name)
    return {"entries_scanned": count, "hits": hits}


def package(runtime, exe_dir, jxl, dependencies, dumpbin, dest, private_roots=()):
    packager = load_packager()
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    if cargo["package"]["version"] != VERSION:
        raise RuntimeError("Cargo version is not the RC2 version")
    if dest.exists():
        raise RuntimeError("Destination must be new")
    manifest = json.loads((ROOT / "RUNTIME-MANIFEST-v1.4.json").read_text(encoding="utf-8"))
    deps = json.loads(dependencies.read_text(encoding="utf-8-sig"))
    if deps["application"] != VERSION:
        raise RuntimeError("Dependency inventory version mismatch")
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    locked = {(p["name"], p["version"]) for p in lock["package"]}
    if not {(p["name"], p["version"]) for p in deps["packages"]} <= locked:
        raise RuntimeError("Dependency inventory is not covered by Cargo.lock")

    dest.mkdir(parents=True)
    payload = dest / "payload"
    shutil.copytree(runtime, payload)
    for name in ("imb-c.exe", "imb-c-cli.exe"):
        copy_new(exe_dir / name, payload / name)
    for name in REPO_FILES:
        copy_new(ROOT / name, payload / name)
    for name in DOCS:
        copy_new(ROOT / "docs" / name, payload / "docs" / name)
    for target, source in LICENSE_EXTRAS.items():
        copy_new(ROOT / source, payload / target)
    copy_new(dependencies, payload / "DEPENDENCIES-v1.4.json")

    checks = {}
    for name, digest in manifest.items():
        if sha(payload / name) != digest:
            raise RuntimeError(f"Runtime mismatch: {name}")
    checks["runtime_manifest_entries"] = len(manifest)
    if (payload / "vcruntime140.dll").exists() or any(p.suffix.lower() == ".dll" for p in payload.iterdir()):
        raise RuntimeError("Application root must not contain DLLs")
    engine_pe = {p.relative_to(payload).as_posix() for p in payload.rglob("*")
                 if p.is_file() and p.suffix.lower() in {".dll", ".exe"} and p.parent != payload}
    unlisted = sorted(n for n in engine_pe if n not in manifest)
    if unlisted:
        raise RuntimeError(f"Executable files outside the runtime manifest: {unlisted}")
    checks["exe_imports"] = {}
    for name in ("imb-c.exe", "imb-c-cli.exe"):
        found = imports(dumpbin, payload / name)
        bad = [d for d in found if d.startswith(CRT_IMPORTS)]
        if bad:
            raise RuntimeError(f"{name} imports redistributable CRT DLLs: {bad}")
        checks["exe_imports"][name] = found
    if not cli_has_no_terms_gate(payload / "imb-c-cli.exe"):
        raise RuntimeError("CLI still exposes the removed runtime-terms command")
    terms_text = (payload / "RUNTIME-RECIPIENT-TERMS.md").read_text(encoding="utf-8")
    if "EULA ID: Cpp_v14_ENU.1033" not in terms_text:
        raise RuntimeError("Shipped Runtime terms lack the official Microsoft text")
    checks["terms"] = {"file": "RUNTIME-RECIPIENT-TERMS.md", "sha256": sha(payload / "RUNTIME-RECIPIENT-TERMS.md")}

    files = inventory(payload)
    (payload / "SHA256SUMS.txt").write_text("".join(f"{d}  {n}\n" for n, d in sorted(files.items())),
                                            encoding="utf-8", newline="\n")
    files["SHA256SUMS.txt"] = sha(payload / "SHA256SUMS.txt")
    portable = [(payload / n, f"IMB-C-v{VERSION}/{n}") for n in sorted(files)]

    names = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard"],
                                    cwd=ROOT, text=True, encoding="utf-8").splitlines()
    names = sorted(n for n in names if n in packager.ROOT_FILES or n in EXTRA_SOURCE_FILES
                   or n.startswith(packager.PREFIXES) or n.startswith(EXTRA_SOURCE_PREFIXES))
    for required in (".cargo/config.toml",
                     "scripts/update-rc2-vc-runtime.py", "scripts/package-rc2-runtime.py",
                     "installer/wix7-v1.4.0-rc.2/make-msi-source.py", "RUNTIME-RECIPIENT-TERMS.md"):
        if required not in names:
            raise RuntimeError(f"Required source is not in the public allowlist: {required}")
    sources = []
    for name in names:
        path = ROOT / name
        if path.lstat().st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT or not path.is_file():
            raise RuntimeError(f"Unexpected source entry: {name}")
        if path.suffix.lower() in {".exe", ".dll", ".pdb", ".pyc", ".msi", ".wixpdb"}:
            raise RuntimeError(f"Binary in source allowlist: {name}")
        sources.append((path, "source/" + name))
    for path in packager.files(payload / "licenses"):
        sources.append((path, "native-materials/" + path.relative_to(payload).as_posix()))
    for path in packager.files(jxl):
        sources.append((path, "native-materials/jxl-rc2/" + path.relative_to(jxl).as_posix()))
    for name in ("RUNTIME-RECIPIENT-TERMS.md", "RUNTIME-HANDOFF.md", "DEPENDENCIES-v1.4.json"):
        sources.append((payload / name, "native-materials/" + name))

    scan = scan_privacy(portable + sources, private_roots)
    (dest / "privacy-scan.json").write_text(json.dumps(scan, indent=2) + "\n", encoding="utf-8")
    if scan["hits"]:
        raise RuntimeError(f"Privacy scan hits: {scan['hits'][:20]}")
    artifacts = dest / "artifacts"
    artifacts.mkdir()
    results = {}
    for kind, entries in (("Portable-x64", portable), ("Source", sources)):
        name = f"IMB-C-v{VERSION}-{kind}.zip"
        results[name] = packager.write_archive(artifacts / name, entries)
    report = {"version": VERSION, "checks": checks, "privacy_entries_scanned": scan["entries_scanned"],
              "payload_files": len(files), "source_files": len(sources), "application_source_files": len(names),
              "archives": results}
    (dest / "package-report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    (dest / "portable-manifest.json").write_text(json.dumps(files, indent=2) + "\n", encoding="utf-8")
    (dest / "source-manifest.json").write_text(json.dumps({n: sha(p) for p, n in sources}, indent=2) + "\n", encoding="utf-8")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for name in ("runtime", "exe-dir", "jxl-materials", "dependencies", "dumpbin", "dest"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--private-root", type=Path, action="append", default=[],
                        help="Additional private location whose path must not appear in any packaged file (repeatable)")
    a = parser.parse_args()
    result = package(a.runtime.resolve(), a.exe_dir.resolve(), a.jxl_materials.resolve(),
                     a.dependencies.resolve(), a.dumpbin.resolve(), a.dest.resolve(), a.private_root)
    print(json.dumps({k: result[k] for k in ("payload_files", "source_files", "archives")}, indent=2))
