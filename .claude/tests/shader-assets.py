#!/usr/bin/env python3
"""A generated corpus is importable only after its run validates it."""

import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "shader_assets", Path(__file__).resolve().parents[2] / "tools/shader-assets.py"
)
assets = importlib.util.module_from_spec(spec)
spec.loader.exec_module(assets)


class FetchTests(unittest.TestCase):
    def test_completed_run_must_succeed_before_its_corpus_is_imported(self):
        for conclusion in ("failure", "cancelled", "success"):
            with self.subTest(conclusion=conclusion), tempfile.TemporaryDirectory() as tmp:
                directory = Path(tmp) / "download"

                def command(argv, **kwargs):
                    if argv[:2] == ["git", "rev-parse"]:
                        output = "a" * 40
                    elif argv[:3] == ["gh", "run", "list"]:
                        output = json.dumps([{
                            "databaseId": 123, "status": "completed",
                            "conclusion": conclusion, "url": "https://example.com/run/123",
                        }])
                    else:
                        self.assertEqual(argv[:3], ["gh", "run", "download"])
                        # Generation can leave an artifact before validation fails.
                        (directory / "generated-metal-assets").mkdir(parents=True)
                        output = ""
                    return subprocess.CompletedProcess(argv, 0, stdout=output)

                with patch.object(assets.subprocess, "run", side_effect=command), \
                        patch.object(assets, "install") as install:
                    if conclusion == "success":
                        assets.fetch(directory)
                        install.assert_called_once_with(directory / "generated-metal-assets")
                    else:
                        with self.assertRaises(SystemExit):
                            assets.fetch(directory)
                        install.assert_not_called()


class VerifyTests(unittest.TestCase):
    def corpus(self, directory, hal):
        options = {"schema": "1", "wgpu-hal": hal, "language": str(3 << 16 | 2), "fast_math": "true",
                   "invariance": "true", "math_mode": "2", "math_functions": "0"}
        files = {"00ab.metal": "kernel void k() {}", "00ab.metallib": "library",
                 "00ab.options": "".join(f"{key}={value}\n" for key, value in options.items())}
        for name, text in files.items():
            (directory / name).write_text(text)
        flags = assets.compiler_flags(options, any_hal=True)
        digests = {name: assets.hashlib.sha256(text.encode()).hexdigest() for name, text in files.items()}
        (directory / "manifest.json").write_text(json.dumps({"schema": 1, "flags": flags, "sha256": digests}))

    def test_a_corpus_from_an_older_wgpu_hal_is_stale_for_the_catalog_not_corrupt(self):
        # A hal bump edits WGPU_HAL, and the committed corpus still records the
        # old one: `verify` on it has to pass, so the workflow reaches the
        # catalog and generates the replacement rather than stopping first.
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            self.corpus(directory, "0.0.1")
            assets.verify(directory, any_hal=True)
            with self.assertRaises(AssertionError):
                assets.verify(directory)
            with patch.object(assets, "EMBEDDED", directory), \
                    patch("sys.argv", ["shader-assets.py", "verify"]):
                assets.main()


if __name__ == "__main__":
    unittest.main()
