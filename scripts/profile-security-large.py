"""Bounded large-image regression with retained logs; never cleans a shared root.

Run a prebuilt Rust test executable. All fixtures must be isolated copies.
Reported disk peaks are sampled visible logical file lengths, not a hard quota
or a measurement of deleted-but-open files. Exit 2 means resource preflight
refused the run; it does not mean a conversion passed.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time


class MemoryStatus(ctypes.Structure):
    _fields_ = [("length", ctypes.c_uint32), ("load", ctypes.c_uint32)] + [
        (name, ctypes.c_uint64) for name in (
            "physical", "available_physical", "commit_limit", "available_commit",
            "virtual", "available_virtual", "extended",
        )
    ]


def memory():
    status = MemoryStatus()
    status.length = ctypes.sizeof(status)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
        raise ctypes.WinError()
    return {name: getattr(status, name) for name, _ in status._fields_}


def visible_size(path):
    total = 0
    try:
        with os.scandir(path) as entries:
            for entry in entries:
                try:
                    info = entry.stat(follow_symlinks=False)
                    if info.st_file_attributes & 0x400:
                        continue
                    if entry.is_dir(follow_symlinks=False):
                        total += visible_size(entry.path)
                    else:
                        total += info.st_size
                except FileNotFoundError:
                    pass
    except FileNotFoundError:
        pass
    return total


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-exe", required=True, type=Path)
    parser.add_argument("--engine", required=True, type=Path)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--side", required=True, type=int)
    parser.add_argument("--format", required=True, choices=("avif", "jxl", "webp"))
    parser.add_argument("--record", required=True, type=Path)
    parser.add_argument("--memory-mib", type=int, help="explicit finite engine Job memory limit for measured calibration")
    args = parser.parse_args()
    # Current calibration is bounded at 100 MP. Do not silently run 200/400 MP.
    if args.side not in (4000, 10000):
        parser.error("only the 16 MP / 100 MP calibrated stages are enabled")
    if args.memory_mib is not None and not 256 <= args.memory_mib <= 65536:
        parser.error("--memory-mib must be 256..65536; disabling limits is forbidden")
    for path in (args.test_exe, args.engine, args.source):
        if not path.is_absolute() or not path.is_file():
            parser.error(f"an absolute existing file is required: {path}")
    if args.source.resolve().is_relative_to(Path(r"D:\don\test")):
        parser.error("copy the read-only original into an isolated fixture directory first")
    args.record.mkdir(parents=True, exist_ok=False)
    root = Path(os.environ.get("MAGICK_TEMPORARY_PATH", r"D:\ImageMagick-Temp"))
    if not root.is_absolute() or not root.is_dir():
        parser.error("temporary root unavailable; no C: fallback")
    with args.source.open("rb") as stream:
        source_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    report = {
        "format": args.format, "side": args.side,
        "source": str(args.source),
        "source_sha256": source_hash,
        "temporary_root": str(root), "memory_before": memory(),
        "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "disk_free_before": shutil.disk_usage(root).free,
        "disk_metric": "sampled visible logical bytes in newly created root entries; 100ms interval",
    }
    requested = ((10240 if args.format == "avif" else 8192 if args.format == "jxl" else 4096)
                 if args.side == 10000 else 3072) * 1048576
    if args.memory_mib is not None:
        requested = args.memory_mib * 1048576
    report["explicit_memory_mib"] = args.memory_mib
    mem = report["memory_before"]
    if requested > mem["physical"] * 3 // 4 or requested + 2 * 1024**3 > mem["available_commit"]:
        report["status"] = "not_run_insufficient_commit"
    elif report["disk_free_before"] < 10 * 1024**3 + args.side**2 * 56 + args.source.stat().st_size:
        report["status"] = "not_run_insufficient_disk"
    else:
        isolated = Path(tempfile.mkdtemp(prefix="imb-c-profile-", dir=root))
        report["isolated_temporary_root"] = str(isolated)
        env = os.environ.copy()
        env.pop("IMBC_ENGINE_MEMORY_MIB", None)
        if args.memory_mib is not None:
            env["IMBC_ENGINE_MEMORY_MIB"] = str(args.memory_mib)
        env.update(IMBC_TEST_ENGINE=str(args.engine), IMBC_TEST_SIDE=str(args.side),
                   IMBC_TEST_FORMAT=args.format, IMBC_TEST_LARGE_SOURCE=str(args.source),
                   IMBC_TEST_RESOURCE_LOG="1")
        # Keep test harness temporary files on D as well; parent environment is unchanged.
        env.update(MAGICK_TEMPORARY_PATH=str(isolated), TEMP=str(isolated), TMP=str(isolated))
        command = [str(args.test_exe), "--exact",
                   "engine::regression_security::security_large_image_profile",
                   "--ignored", "--nocapture", "--test-threads=1"]
        report["command"] = command
        started = time.monotonic()
        peak = 0
        observation_errors = []
        with (args.record / "test.log").open("wb") as log:
            child = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT)
            report["pid"] = child.pid
            while child.poll() is None:
                try:
                    peak = max(peak, visible_size(isolated))
                except OSError as error:
                    if len(observation_errors) < 10:
                        observation_errors.append(str(error))
                if time.monotonic() - started > 690:
                    child.kill()  # OS closes the test parent's non-inherited Job Object handles.
                    report["timeout"] = True
                    break
                time.sleep(0.1)
            report["exit_code"] = child.wait()
        text = (args.record / "test.log").read_text(encoding="utf-8", errors="replace")
        report.update(
            elapsed_seconds=time.monotonic() - started,
            observed_logical_temporary_peak=peak,
            observation_errors=observation_errors,
            peak_engine_job_commit=max(map(int, re.findall(r"peak_job_commit=(\d+)", text)), default=0),
            # 2026-11-13：每次引擎執行的核心 Job 寫入量扣除標準串流，用於校準 32 GiB 原生寫入上限。
            peak_native_job_writes=max(
                (int(w) - int(p) for w, p in re.findall(r"job_write_bytes=(\d+) piped=(\d+)", text)),
                default=0,
            ),
            observed_native_peak=max(map(int, re.findall(r"observed_peak_bytes=(\d+)", text)), default=0),
            job_limit_failure_reported="committed-memory" in text or "0xe06d7363" in text,
            status="passed" if report["exit_code"] == 0 and "1 passed; 0 failed" in text else "failed",
        )
    report["memory_after"] = memory()
    report["disk_free_after"] = shutil.disk_usage(root).free
    with args.source.open("rb") as stream:
        report["source_sha256_after"] = hashlib.file_digest(stream, "sha256").hexdigest()
    assert report["source_sha256"] == report["source_sha256_after"], "source changed"
    (args.record / "metrics.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2), flush=True)
    return 0 if report["status"] == "passed" else 2


if __name__ == "__main__":
    raise SystemExit(main())
