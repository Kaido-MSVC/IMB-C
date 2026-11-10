"""Small simulated runtime inventories; never touches the installed runtime."""
import hashlib
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("prepare_runtime", Path(__file__).resolve().parents[1] / "scripts/prepare-v13-runtime.py")
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)

class RuntimePreflightTests(unittest.TestCase):
    def test_invalid_late_entry_does_not_copy_earlier_entry(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source, dest = root / "source", root / "destination"
            source.mkdir(); dest.mkdir()
            (source / "licenses").mkdir()
            (source / "first").write_bytes(b"new")
            (dest / "first").write_bytes(b"original")
            manifest = {"first": hashlib.sha256(b"new").hexdigest(), "missing": "invalid"}
            with self.assertRaises(RuntimeError):
                runtime.prepare(source, dest, manifest)
            self.assertEqual((dest / "first").read_bytes(), b"original")
            self.assertEqual(list(dest.iterdir()), [dest / "first"])

    def test_copy_and_postflight_and_mismatch(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source, dest = root / "source", root / "destination"
            source.mkdir(); (source / "licenses").mkdir()
            (source / "engine").write_bytes(b"runtime")
            manifest = {"engine": hashlib.sha256(b"runtime").hexdigest()}
            runtime.prepare(source, dest, manifest)
            runtime.verify(dest, manifest)
            (dest / "engine").write_bytes(b"corrupt")
            with self.assertRaisesRegex(RuntimeError, "Mismatched"):
                runtime.verify(dest, manifest)

if __name__ == "__main__":
    unittest.main()
