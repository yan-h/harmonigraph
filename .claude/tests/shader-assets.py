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


if __name__ == "__main__":
    unittest.main()
