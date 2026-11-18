"""以 Visual Studio 正式版（default，非 preview）Redist 更新 RC2 隨包的 VC++ Runtime。

只替換 ImageMagick 目錄內四份 Microsoft DLL，移除程式根目錄的 vcruntime140.dll
（GUI／CLI 改以靜態 CRT 建置），其餘 runtime 檔案逐一核對舊清單後原樣複製。

用法：
  python -X utf8 scripts/update-rc2-vc-runtime.py --baseline <已驗收 RC2 stage>
      --old-manifest <舊 RUNTIME-MANIFEST-v1.4.json> --vs <Visual Studio 安裝根目錄>
      --dest <新目錄> --manifest-out <新清單> --provenance-out <來源證明 JSON>
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import struct
import subprocess

REPLACED = {
    "ImageMagick/MSVCP140.dll": ("CRT", "msvcp140.dll"),
    "ImageMagick/vcruntime140.dll": ("CRT", "vcruntime140.dll"),
    "ImageMagick/vcruntime140_1.dll": ("CRT", "vcruntime140_1.dll"),
    "ImageMagick/vcomp140.dll": ("OpenMP", "vcomp140.dll"),
}
REMOVED = {"vcruntime140.dll"}
# 由建置產生或由專案提供的檔案，不從基線複製。
NOT_COPIED = {
    "imb-c.exe", "imb-c-cli.exe", "vcruntime140.dll", "RUNTIME-MANIFEST-v1.4.json",
    "README.md", "LICENSE", "THIRD-PARTY-NOTICES.md", "THIRD-PARTY-RUNTIME.txt", "SHA256SUMS.txt",
}


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def regular(path, root):
    for entry in (path, *[p for p in path.parents if root in p.parents or p == root]):
        if entry.lstat().st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT:
            raise RuntimeError(f"Reparse point: {entry}")
    if not path.is_file():
        raise RuntimeError(f"Missing file: {path}")


def file_version(path):
    version = ctypes.windll.version
    size = version.GetFileVersionInfoSizeW(str(path), None)
    if not size:
        raise RuntimeError(f"No version resource: {path}")
    buffer = ctypes.create_string_buffer(size)
    if not version.GetFileVersionInfoW(str(path), 0, size, buffer):
        raise RuntimeError(f"Cannot read version: {path}")
    pointer, length = ctypes.c_void_p(), ctypes.c_uint()
    version.VerQueryValueW(buffer, "\\", ctypes.byref(pointer), ctypes.byref(length))
    fixed = ctypes.string_at(pointer.value, length.value)
    signature, _, ms, ls, _, _, mask, flags = struct.unpack_from("<8I", fixed)
    if signature != 0xFEEF04BD:
        raise RuntimeError(f"Invalid version resource: {path}")
    flags &= mask
    return (ms >> 16, ms & 0xFFFF, ls >> 16, ls & 0xFFFF), bool(flags & 1), bool(flags & 2)


def linker(path):
    data = path.read_bytes()[:4096]
    if data[:2] != b"MZ":
        return None
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        return None
    return data[pe + 26], data[pe + 27]


def signatures(paths):
    script = (
        "$ErrorActionPreference='Stop'; @($env:IMBC_SIGNATURE_PATHS -split \"`n\") | ForEach-Object { "
        "$s = Get-AuthenticodeSignature -LiteralPath $_; "
        "[pscustomobject]@{Path=$_; Status=[string]$s.Status; Subject=$s.SignerCertificate.Subject; "
        "Issuer=$s.SignerCertificate.Issuer; Timestamped=[bool]$s.TimeStamperCertificate} } | ConvertTo-Json -Depth 3"
    )
    env = dict(os.environ, IMBC_SIGNATURE_PATHS="\n".join(map(str, paths)))
    out = subprocess.run(["pwsh", "-NoProfile", "-NonInteractive", "-Command", script], env=env,
                         check=True, capture_output=True, text=True, encoding="utf-8").stdout
    rows = json.loads(out)
    return {Path(r["Path"]): r for r in (rows if isinstance(rows, list) else [rows])}


def vswhere(vs_root):
    tool = Path(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe")
    rows = json.loads(subprocess.run([str(tool), "-all", "-prerelease", "-format", "json", "-utf8"],
                                     check=True, capture_output=True, text=True, encoding="utf-8").stdout)
    for row in rows:
        if Path(row["installationPath"]).resolve() == vs_root.resolve():
            return row
    raise RuntimeError("Visual Studio installation not found by vswhere")


def update(baseline, old_manifest, vs_root, dest, manifest_out, provenance_out):
    old = json.loads(old_manifest.read_text(encoding="utf-8"))
    for name, digest in old.items():
        regular(baseline / name, baseline)
        if sha(baseline / name) != digest:
            raise RuntimeError(f"Baseline does not match the old manifest: {name}")
    missing = (set(REPLACED) | REMOVED) - set(old)
    if missing:
        raise RuntimeError(f"Old manifest lacks expected entries: {sorted(missing)}")

    instance = vswhere(vs_root)
    if instance.get("isPrerelease") or not instance.get("channelId", "").endswith(".Release"):
        raise RuntimeError("Only a Visual Studio Release (Stable) channel installation is accepted")
    build = vs_root / "VC/Auxiliary/Build"
    redist_version = (build / "Microsoft.VCRedistVersion.default.txt").read_text(encoding="utf-8").strip()
    tools_version = (build / "Microsoft.VCToolsVersion.default.txt").read_text(encoding="utf-8").strip()
    preview_file = build / "Microsoft.VCToolsVersion.Preview.txt"
    preview = preview_file.read_text(encoding="utf-8").strip() if preview_file.is_file() else None
    if redist_version == preview:
        raise RuntimeError("Default redistributable version equals the preview toolset; refusing")
    redist = vs_root / "VC/Redist/MSVC" / redist_version / "x64"
    crt = [p for p in redist.glob("Microsoft.VC14*.CRT") if p.is_dir()]
    omp = [p for p in redist.glob("Microsoft.VC14*.OpenMP") if p.is_dir()]
    if len(crt) != 1 or len(omp) != 1:
        raise RuntimeError("Unexpected redistributable folder layout")
    folders = {"CRT": crt[0], "OpenMP": omp[0]}

    sources = {}
    for target, (kind, name) in REPLACED.items():
        source = folders[kind] / name
        regular(source, vs_root)
        if any(part.lower() in {"debug_nonredist", "onecore"} for part in source.parts):
            raise RuntimeError(f"Not a redistributable location: {source}")
        sources[target] = source
    sigs = signatures(sources.values())
    replacement_versions = set()
    records = {}
    for target, source in sources.items():
        version, debug, prerelease = file_version(source)
        sig = sigs[source]
        if debug or prerelease:
            raise RuntimeError(f"Debug or prerelease file: {source}")
        if sig["Status"] != "Valid" or "O=Microsoft Corporation" not in (sig["Subject"] or "") or not sig["Timestamped"]:
            raise RuntimeError(f"Microsoft signature not valid: {source} {sig}")
        replacement_versions.add(version)
        records[target] = {
            "source": "[VisualStudioFolder]/" + source.relative_to(vs_root).as_posix(),
            "file_version": ".".join(map(str, version)),
            "sha256": sha(source),
            "previous_sha256": old[target],
            "signer": sig["Subject"].split(",")[0],
            "issuer": sig["Issuer"].split(",")[0],
            "authenticode": sig["Status"],
            "timestamped": sig["Timestamped"],
        }
    if len(replacement_versions) != 1:
        raise RuntimeError(f"Replacement DLLs disagree on version: {replacement_versions}")
    runtime_version = replacement_versions.pop()

    if dest.exists():
        raise RuntimeError("Destination must be new")
    dest.mkdir(parents=True)
    copied = 0
    for path in sorted(baseline.rglob("*")):
        rel = path.relative_to(baseline).as_posix()
        if path.lstat().st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT:
            raise RuntimeError(f"Reparse point: {path}")
        if not path.is_file() or rel in NOT_COPIED or rel in REPLACED:
            continue
        if rel in old and sha(path) != old[rel]:
            raise RuntimeError(f"Changed while copying: {rel}")
        target = dest / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)
        copied += 1
    for target, source in sources.items():
        shutil.copyfile(source, dest / target)
        if sha(dest / target) != records[target]["sha256"]:
            raise RuntimeError(f"Copy verification failed: {target}")

    newest = (0, 0)
    pe_count = 0
    for path in sorted(dest.rglob("*")):
        if path.is_file() and path.suffix.lower() in {".dll", ".exe"} and path.relative_to(dest).as_posix() not in REPLACED:
            version = linker(path)
            if version:
                pe_count += 1
                newest = max(newest, version)
    if runtime_version[:2] < newest:
        raise RuntimeError(f"Runtime {runtime_version} is older than linker {newest}")

    manifest = {}
    for name in old:
        if name in REMOVED:
            continue
        manifest[name] = records[name]["sha256"] if name in records else old[name]
        if sha(dest / name) != manifest[name]:
            raise RuntimeError(f"New runtime mismatch: {name}")
    manifest_out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8", newline="\n")
    provenance = {
        "purpose": "IMB-C 1.4.0-rc.2 Microsoft Visual C++ Runtime update (runtime must not be older than the build tools)",
        "visual_studio": {
            "product": instance.get("displayName"),
            "installation_version": instance.get("installationVersion"),
            "channel_id": instance.get("channelId"),
            "is_prerelease": instance.get("isPrerelease"),
            "vc_redist_default": redist_version,
            "vc_tools_default": tools_version,
            "vc_tools_preview_not_used": preview,
        },
        "redistribution_basis": "Visual Studio 2026 Distributable Code: [VisualStudioFolder]\\VC\\redist, unmodified; not debug_nonredist, not onecore, not preview",
        "runtime_file_version": ".".join(map(str, runtime_version)),
        "newest_bundled_linker": f"{newest[0]}.{newest[1]}",
        "bundled_pe_checked": pe_count,
        "files": records,
        "removed": {name: {"previous_sha256": old[name], "reason": "imb-c.exe and imb-c-cli.exe use the static CRT"} for name in sorted(REMOVED)},
        "manifest_entries": len(manifest),
        "previous_manifest_entries": len(old),
        "copied_unchanged_files": copied,
    }
    provenance_out.write_text(json.dumps(provenance, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return provenance


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for name in ("baseline", "old-manifest", "vs", "dest", "manifest-out", "provenance-out"):
        parser.add_argument("--" + name, type=Path, required=True)
    a = parser.parse_args()
    result = update(a.baseline.resolve(), a.old_manifest.resolve(), a.vs.resolve(), a.dest.resolve(),
                    a.manifest_out.resolve(), a.provenance_out.resolve())
    print(json.dumps({k: result[k] for k in ("runtime_file_version", "newest_bundled_linker", "manifest_entries", "copied_unchanged_files")}, indent=2))
