"""Optional real CLI regression; no private runtime or iat required by CI."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import shutil
import unittest
import zlib

CLI = os.environ.get("IMBC_TEST_CLI", "")
ROOT = Path(__file__).resolve().parents[1]
_CONFIG = None


def setUpModule():
    """Keep the CLI's settings folder away from the user's real preferences."""
    global _CONFIG
    if not (CLI and Path(CLI).is_file()):
        return
    _CONFIG = tempfile.TemporaryDirectory()
    os.environ["IMBC_CONFIG_DIR"] = _CONFIG.name


def tearDownModule():
    if _CONFIG is not None:
        os.environ.pop("IMBC_CONFIG_DIR", None)
        _CONFIG.cleanup()


@unittest.skipUnless(CLI and Path(CLI).is_file(), "set IMBC_TEST_CLI")
class RuntimeNoticeTests(unittest.TestCase):
    """The bundled Microsoft Runtime terms are documentation; they never gate conversion."""

    def run_cli(self, *args):
        return subprocess.run([CLI, *map(str, args)], capture_output=True, encoding="utf-8",
                              timeout=60, cwd=ROOT)

    def test_removed_terms_commands_are_rejected(self):
        self.assertEqual(self.run_cli("runtime-terms", "status").returncode, 2)
        self.assertNotIn("runtime-terms", self.run_cli("--help").stdout)
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_cli("convert", ROOT / "tests/fixtures/acceptance-v13/icc8.png", "--format", "png",
                                  "--output-dir", temp, "--no-iat", "--accept-runtime-terms", "0" * 64)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(list(Path(temp).iterdir()), [])

    def test_conversion_needs_no_acceptance_record_and_keeps_old_record(self):
        config = Path(os.environ["IMBC_CONFIG_DIR"])
        legacy = config / "runtime-terms-acceptance.json"
        legacy.write_bytes(b'{"schema":1}')
        source = ROOT / "tests/fixtures/acceptance-v13/icc8.png"
        before = source.read_bytes()
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_cli("convert", source, "--format", "png", "--output-dir", temp, "--no-iat")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["status"], "completed")
        self.assertEqual(source.read_bytes(), before)
        self.assertEqual(legacy.read_bytes(), b'{"schema":1}')
        legacy.unlink()

    def test_terms_file_ships_with_official_text(self):
        shipped = Path(CLI).with_name("RUNTIME-RECIPIENT-TERMS.md")
        if not shipped.is_file():
            shipped = ROOT / "RUNTIME-RECIPIENT-TERMS.md"
        text = shipped.read_text(encoding="utf-8")
        self.assertIn("MICROSOFT VISUAL C++ V14 REDISTRIBUTABLE and RUNTIME", text)
        self.assertIn("EULA ID: Cpp_v14_ENU.1033", text)


@unittest.skipUnless(CLI and Path(CLI).is_file(), "set IMBC_TEST_CLI")
class CliTests(unittest.TestCase):
    def run_cli(self, *args):
        return subprocess.run([CLI, *map(str, args)], capture_output=True,
                              encoding="utf-8", timeout=60, cwd=ROOT)

    def test_help_version_and_errors(self):
        self.assertEqual(self.run_cli("--help").returncode, 0)
        self.assertIn("Usage:", self.run_cli("convert", "--help").stdout)
        self.assertIn("IMB-C", self.run_cli("--version").stdout)
        self.assertEqual(self.run_cli("convert").returncode, 2)

    def test_lossless_quality_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_cli("convert", ROOT / "tests/fixtures/acceptance-v13/icc8.png",
                                  "--format", "avif", "--mode", "lossless", "--quality", "75", "--output-dir", temp)
            self.assertEqual(result.returncode, 2)
            self.assertIn("--quality", result.stderr)
            self.assertEqual(list(Path(temp).iterdir()), [])

    def test_missing_explicit_iat_keeps_original_conversion(self):
        with tempfile.TemporaryDirectory() as temp:
            source = ROOT / "tests/fixtures/acceptance-v13/icc8.png"
            result = self.run_cli("convert", source, "--format", "png", "--output-dir", temp,
                                  "--iat", Path(temp) / "missing.exe")
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertIn("iat", result.stderr)
            row = json.loads(result.stdout)
            self.assertEqual(row["iat"]["verdict"], "unavailable")
            self.assertTrue(row["iat_requested"])
            self.assertFalse(row["iat_available"])
            self.assertTrue(Path(row["output"]).is_file())

    @unittest.skipUnless(os.environ.get("IMBC_TEST_OLD_IAT"), "set IMBC_TEST_OLD_IAT")
    def test_incompatible_iat_keeps_conversion_and_existing_verification(self):
        with tempfile.TemporaryDirectory() as temp:
            source = ROOT / "tests/fixtures/acceptance-v13/icc8.png"
            result = self.run_cli("convert", source, "--format", "png", "--output-dir", temp,
                                  "--iat", os.environ["IMBC_TEST_OLD_IAT"])
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertIn("0.5.1", result.stderr)
            row = json.loads(result.stdout)
            self.assertEqual(row["iat"]["verdict"], "unavailable")
            self.assertTrue(row["iat_requested"])
            self.assertFalse(row["iat_available"])
            self.assertTrue(row["pixels_exact"])
            self.assertTrue(row["icc_verified"])
            self.assertTrue(Path(row["output"]).is_file())

    @unittest.skipUnless(os.name == "nt", "Windows UTF-16 paths")
    def test_unpaired_surrogate_does_not_abort_batch_or_report(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "source-\ud800.png"
            original = ROOT / "tests/fixtures/acceptance-v13/icc8.png"
            source.write_bytes(original.read_bytes())
            before = (source.stat().st_size, source.stat().st_mtime_ns, hashlib.sha256(source.read_bytes()).hexdigest())
            report = Path(temp) / "summary.json"
            result = self.run_cli("convert", "--input", source, "--input", original,
                                  "--format", "png", "--output-dir", temp, "--report-json", report, "--no-iat")
            self.assertEqual(result.returncode, 0, result.stderr)
            rows = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertEqual(len(rows), 2)
            self.assertEqual(len(json.loads(report.read_text(encoding="utf-8"))["results"]), 2)
            self.assertTrue(all(row["status"] == "completed" for row in rows))
            self.assertEqual(before, (source.stat().st_size, source.stat().st_mtime_ns, hashlib.sha256(source.read_bytes()).hexdigest()))

    def test_gray_avif_lossless_is_rejected_without_output(self):
        source = ROOT / "tests/fixtures/gray-v12.png"
        before = source.read_bytes()
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_cli("convert", source, "--format", "avif", "--mode", "lossless", "--output-dir", temp, "--no-iat")
            self.assertEqual(result.returncode, 1)
            self.assertIn("RGB", json.loads(result.stdout)["error"])
            self.assertEqual(list(Path(temp).iterdir()), [])
            self.assertEqual(source.read_bytes(), before)

    def test_batch_failure_preserves_sources_and_outputs(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            original = ROOT / "tests/fixtures/acceptance-v13/icc8.png"
            before = hashlib.sha256(original.read_bytes()).digest()
            broken = folder / "broken.png"
            broken.write_bytes(b"invalid")
            sentinel = folder / "icc8.png"
            sentinel.write_bytes(b"existing output")
            report = folder / "summary.json"
            result = self.run_cli("convert", original, broken, "--format", "png",
                                  "--output-dir", folder, "--no-iat", "--report-json", report)
            self.assertEqual(result.returncode, 1, result.stderr)
            rows = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertEqual(sorted(row["status"] for row in rows), ["completed", "failed"])
            self.assertEqual(json.loads(report.read_text())["exit_code"], 1)
            self.assertEqual(hashlib.sha256(original.read_bytes()).digest(), before)
            self.assertEqual(sentinel.read_bytes(), b"existing output")
            self.assertEqual(broken.read_bytes(), b"invalid")
            self.assertEqual(self.run_cli("convert", original, "--format", "png", "--output-dir", folder,
                                          "--no-iat", "--report-json", report).returncode, 2)

    def test_jxl_lossy_keeps_exif_and_xmp(self):
        original = ROOT / "tests/fixtures/acceptance-v13/baseline-metadata.jpg"
        before = original.read_bytes()
        metadata = {}
        at = 2
        while at < len(before):
            self.assertEqual(before[at], 255)
            marker = before[at + 1]
            if marker == 218:
                break
            size = int.from_bytes(before[at + 2:at + 4], "big")
            payload = before[at + 4:at + 2 + size]
            if marker == 225:
                if payload.startswith(b"Exif\0\0"):
                    metadata[b"Exif"] = payload[6:]
                if payload.startswith(b"http://ns.adobe.com/xap/1.0/\0"):
                    metadata[b"xml "] = payload.split(b"\0", 1)[1]
            at += 2 + size
        self.assertEqual(set(metadata), {b"Exif", b"xml "})
        with tempfile.TemporaryDirectory() as temp:
            # Exercise metadata independently of JPEG profiles which the bundled
            # lossy encoder may replace (the engine must safely reject those).
            png = (ROOT / "tests/fixtures/acceptance-v13/icc8.png").read_bytes()
            def chunk(kind, payload):
                return len(payload).to_bytes(4, "big") + kind + payload + zlib.crc32(kind + payload).to_bytes(4, "big")
            source = Path(temp) / "metadata.png"
            source.write_bytes(png[:-12] + chunk(b"eXIf", metadata[b"Exif"]) + chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0" + metadata[b"xml "]) + png[-12:])
            result = self.run_cli("convert", source, "--format", "jxl", "--mode", "lossy", "--quality", "75", "--output-dir", temp, "--no-iat")
            self.assertEqual(result.returncode, 0, result.stderr)
            data = Path(json.loads(result.stdout)["output"]).read_bytes()
            found = {}
            at = 0
            while at < len(data):
                size = int.from_bytes(data[at:at + 4], "big")
                kind = data[at + 4:at + 8]
                header = 8
                if size == 1:
                    size = int.from_bytes(data[at + 8:at + 16], "big")
                    header = 16
                if size == 0:
                    size = len(data) - at
                self.assertGreaterEqual(size, header)
                self.assertLessEqual(at + size, len(data))
                payload = data[at + header:at + size]
                if kind == b"Exif":
                    offset = int.from_bytes(payload[:4], "big")
                    found[kind] = payload[4 + offset:]
                if kind == b"xml ":
                    found[kind] = payload
                at += size
            self.assertEqual(found, metadata)
            self.assertEqual(original.read_bytes(), before)

    def test_jxl_lossy_icc_replacement_is_safely_rejected(self):
        original = ROOT / "tests/fixtures/acceptance-v13/baseline-metadata.jpg"
        before = original.read_bytes()
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_cli("convert", original, "--format", "jxl", "--mode", "lossy", "--quality", "75", "--output-dir", temp, "--no-iat")
            self.assertEqual(result.returncode, 1)
            self.assertIn("ICC", json.loads(result.stdout)["error"])
            self.assertEqual(list(Path(temp).iterdir()), [])
            self.assertEqual(original.read_bytes(), before)

    @unittest.skipUnless(os.environ.get("IMBC_TEST_IAT"), "set IMBC_TEST_IAT")
    def test_optional_iat_avif_proof_and_jxl_difference_keep_output(self):
        with tempfile.TemporaryDirectory() as temp:
            original = ROOT / "tests/fixtures/acceptance-v13/icc8.png"
            for fmt, mode in [("avif", "lossless"), ("jxl", "lossy")]:
                quality = ["--quality", "75"] if mode == "lossy" else []
                result = self.run_cli("convert", original, "--format", fmt, "--mode", mode,
                                      *quality, "--output-dir", temp, "--iat", os.environ["IMBC_TEST_IAT"])
                self.assertEqual(result.returncode, 0, result.stderr)
                row = json.loads(result.stdout)
                self.assertTrue(Path(row["output"]).is_file())
                if fmt == "avif":
                    self.assertEqual(row["iat"]["code"], 0)
                    self.assertTrue(row["iat"]["avif_pixel_verified"])
                else:
                    self.assertIn(row["iat"]["code"], (1, 2))


@unittest.skipUnless(CLI and Path(CLI).is_file() and shutil.which("rustc"), "set IMBC_TEST_CLI; rustc required")
class Rc2ContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = tempfile.TemporaryDirectory()
        cls.tool = Path(cls.fixture.name) / "iat.exe"
        subprocess.run(["rustc", "--edition=2024", "-C", "debuginfo=0", str(ROOT / "tests/helpers/iat_stub.rs"), "-o", str(cls.tool)], check=True, capture_output=True)
        cls.tool.with_name("iat-worker.exe").write_bytes(b"inert fixture, never executed")

    @classmethod
    def tearDownClass(cls):
        cls.fixture.cleanup()

    def invoke(self, mode, folder):
        env = os.environ.copy()
        env["IMBC_STUB_MODE"] = mode
        return subprocess.run([CLI, "convert", str(ROOT / "tests/fixtures/acceptance-v13/icc8.png"),
                               "--format", "avif", "--mode", "lossless", "--output-dir", folder,
                               "--iat", str(self.tool), "--report-json", str(Path(folder) / "summary.json")],
                              capture_output=True, encoding="utf-8", env=env, timeout=60)

    def assert_case(self, mode, outer_code, verdict, code, tool_code):
        with tempfile.TemporaryDirectory() as folder:
            result = self.invoke(mode, folder)
            self.assertEqual(result.returncode, outer_code, result.stderr)
            row = json.loads(result.stdout)
            self.assertEqual(row["status"], "completed")
            self.assertEqual(row["iat"]["verdict"], verdict)
            self.assertEqual(row["iat"]["code"], code)
            self.assertEqual(row["iat"]["tool_exit_code"], tool_code)
            self.assertEqual(row["iat"]["verified"], verdict == "verified")
            self.assertTrue(Path(row["output"]).is_file())
            summary = json.loads((Path(folder) / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(summary["exit_code"], outer_code)
            self.assertTrue(summary["iat_requested"])
            self.assertEqual(summary["results"][0], row)

    def test_avif_verified(self):
        self.assert_case("verified", 0, "verified", 0, 0)

    def test_avif_missing_proof_is_indeterminate(self):
        self.assert_case("indeterminate", 0, "indeterminate", 2, 0)

    def test_avif_different_is_not_conversion_corruption(self):
        self.assert_case("different", 0, "different", 1, 1)

    def test_avif_decode_failure_is_explicit(self):
        self.assert_case("invalid", 0, "invalid", 3, 3)

    def test_requested_tool_execution_error_is_exit_two(self):
        self.assert_case("tool_error", 2, "tool_error", 5, 5)

    def test_bad_schema_is_tool_error_without_deleting_output(self):
        self.assert_case("bad_schema", 2, "tool_error", None, None)

    def test_closed_stdout_help_version_and_gui_subsystem_do_not_panic(self):
        for exe, args in [(Path(CLI), ["--help"]), (Path(CLI), ["convert", "a.png", "--help"]),
                          (Path(CLI), ["--version"]), (Path(CLI).with_name("imb-c.exe"), ["--version"])]:
            if not exe.is_file():
                continue
            read, write = os.pipe()
            os.close(read)
            try:
                child = subprocess.Popen([exe, *args], stdout=write, stderr=subprocess.PIPE)
            finally:
                os.close(write)
            _, error = child.communicate(timeout=15)
            self.assertEqual(child.returncode, 0, error)

    def test_decode_diagnostics_do_not_include_legal_banner(self):
        with tempfile.TemporaryDirectory() as folder:
            result = subprocess.run([CLI, "convert", str(ROOT / "tests/fixtures/invalid-idat-v121.png"),
                                     "--format", "png", "--output-dir", folder, "--no-iat"], capture_output=True, encoding="utf-8", timeout=60)
            self.assertEqual(result.returncode, 1)
            error = json.loads(result.stdout)["error"]
            self.assertIn("IDAT", error)
            self.assertNotIn("IMB-C distribution: linked LGPL", error)
            self.assertEqual(list(Path(folder).iterdir()), [])


if __name__ == "__main__":
    unittest.main()
