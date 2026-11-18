"""Stage the fixed upstream sources and vcpkg ports used by the RC2 JXL tools."""

import argparse
import hashlib
from pathlib import Path, PurePosixPath
import shutil
import zipfile


ARCHIVES = {
    "libjxl-0.13.0-brotli-highway-source.zip": (
        "", "", "9ae1075545921c140a0cdbd58b584e60c8a99ce63835bd068a78a87e5a91c638"
    ),
    "sjpeg-dd4d9857.zip": (
        "sjpeg-dd4d9857c58a25713711a46d37385ad23ca2177e/", "libjxl-0.13.0/third_party/sjpeg",
        "ffe06a318a4f4ad404ed976b3edf63f739e76e1ea67f360e31f57acd08011368"
    ),
    "skcms-96d9171c.zip": (
        "skcms-96d9171c94b937a1b5f0293de7309ac16311b722/", "libjxl-0.13.0/third_party/skcms",
        "e6dda76c35b4f5833e17a39ee5e43fd827fd447d5103e3db355898a29609a26f"
    ),
    "lcms-51763476.zip": (
        "Little-CMS-5176347635785e53ee5cee92328f76fda766ecc6/", "libjxl-0.13.0/third_party/lcms",
        "aa1f48ea4b207cb4ccccc9a3fd4cc6753669f0435a4ef6336fcd342e6bf13cd7"
    ),
    "vcpkg-2025.10.17.zip": (
        "vcpkg-2025.10.17/", "vcpkg",
        "0ddaf1430da978e6c944eae37e29f06887b1f101d8b322eaffebc0cf2e6b461f"
    ),
}


def extract(path, prefix, dest):
    seen = set()
    with zipfile.ZipFile(path) as bundle:
        for item in bundle.infolist():
            raw = item.filename
            if item.is_dir():
                continue
            name = PurePosixPath(raw.removeprefix(prefix))
            if (not raw.startswith(prefix) or name.is_absolute() or ".." in name.parts
                    or ":" in raw or "\\" in raw or (item.external_attr >> 28) == 10
                    or raw.casefold() in seen):
                raise ValueError("Unsafe archive path: " + raw)
            seen.add(raw.casefold())
            target = dest.joinpath(*name.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.open(item) as src, target.open("xb") as out:
                shutil.copyfileobj(src, out)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archives", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True, help="A new directory at a short local path")
    args = parser.parse_args()
    for filename, (_, _, expected) in ARCHIVES.items():
        with (args.archives / filename).open("rb") as source:
            if hashlib.file_digest(source, "sha256").hexdigest() != expected:
                raise ValueError("Unexpected SHA-256: " + filename)
    args.work.mkdir(parents=True)
    for filename, (prefix, subdirectory, _) in ARCHIVES.items():
        extract(args.archives / filename, prefix, args.work / subdirectory)


if __name__ == "__main__":
    main()
