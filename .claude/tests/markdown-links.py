#!/usr/bin/env python3
"""Local link fixtures use real files and a temporary Git index; never the repo."""

import runpy
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "markdown-links.py"
checker = runpy.run_path(str(SCRIPT))


class MarkdownLinksTests(unittest.TestCase):
    def check(self, files):
        with tempfile.TemporaryDirectory(prefix="markdown-links-") as tmp:
            root = Path(tmp)
            for path, content in files.items():
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                (root / path).write_text(content)
            return checker["check"](root, set(files))

    def test_inline_images_directories_and_local_or_cross_document_anchors(self):
        files = {
            "README.md": "# Home\n[Guide](docs/guide.md#voice-identity)\n![Picture](docs/picture.png)\n[Directory](docs)\n[Here](#home)\n",
            "docs/guide.md": "# Voice identity\n> [Home](../README.md#home)\n[Root](..)\n[Source](../lib.rs#L1)\n",
            "docs/picture.png": "image fixture",
            "lib.rs": "// source line\n",
        }
        self.assertEqual(self.check(files), [])
        files["README.md"] += "[Missing](gone.md)\n[Stale](docs/guide.md#old-heading)\n"
        errors = self.check(files)
        self.assertEqual(len(errors), 2, errors)
        self.assertIn("README.md:6: missing tracked target", errors[0])
        self.assertIn("README.md:7: missing heading anchor", errors[1])

    def test_fences_inline_examples_and_comments_do_not_create_links_or_anchors(self):
        files = {"README.md": """# Real
```markdown
# Fake
[Missing](fenced.md)
```
  ~~~~markdown
[Missing](tilde.md)
```
~~~
[Missing](still-fenced.md)
  ~~~~
    [Missing](indented.md)
`[Missing](inline.md)` and `` `[Missing](backtick.md)` ``
<!-- [Missing](comment.md)
# Comment heading
```
-->
[A `code` label](#real)
[Stale](#fake)
[Missing](after-fence.md)
"""}
        errors = self.check(files)
        self.assertEqual(len(errors), 2, errors)
        self.assertIn("missing heading anchor: '#fake'", errors[0])
        self.assertIn("after-fence.md", errors[1])

    def test_heading_punctuation_code_unicode_and_duplicate_ids(self):
        text = """# #615 Bitwig timing apparatus
## Lattice history ownership (#645 B)
### SG1 — a `node_key`
# Café &amp; tools
Setext title
============
## Repeat
## Repeat
## Repeat-1
"""
        expected = {"615-bitwig-timing-apparatus", "lattice-history-ownership-645-b",
                    "sg1--a-node_key", "café--tools", "setext-title",
                    "repeat", "repeat-1", "repeat-1-1"}
        self.assertEqual(checker["anchors"](text), expected)
        text += "\n".join(f"[Heading](#{slug})" for slug in expected)
        self.assertEqual(self.check({"README.md": text}), [])

    def test_url_escaping_balanced_parentheses_titles_and_reference_destinations(self):
        files = {
            "README.md": r"""[Guide](docs/a%20b.md?raw=1#caf%C3%A9)
[Angle](<docs/a b.md#café> "a title")
[Parentheses](docs/a(b).md)
[Escaped](docs/a\(b\).md)
[Hash](docs/hash%23name.md)
[Ampersand](docs/a&amp;b.md)
[Reference][guide]
[guide]: docs/a%20b.md#caf%C3%A9 "title"
[External](https://example.invalid/missing.md#missing)
[Email](mailto:nobody@example.invalid)
[Absolute](/site/path.md)
[Protocol relative](//example.invalid/missing.md)
""",
            "docs/a b.md": "# Café\n",
            "docs/a(b).md": "# Parentheses\n",
            "docs/hash#name.md": "# Hash\n",
            "docs/a&b.md": "# Ampersand\n",
        }
        self.assertEqual(self.check(files), [])
        files["README.md"] += "[missing]: docs/gone.md\n"
        self.assertEqual(len(self.check(files)), 1)

    def test_cli_rejects_present_untracked_targets_and_ignores_vendor_and_symlink_sources(self):
        with tempfile.TemporaryDirectory(prefix="markdown-links-") as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-q", tmp], check=True)
            (root / "README.md").write_text("[New](new.md#new)\n")
            (root / "new.md").write_text("# New\n")
            (root / "vendor").mkdir()
            (root / "vendor/README.md").write_text("[Missing](gone.md)\n")
            (root / "AGENTS.md").symlink_to("vendor/README.md")
            subprocess.run(["git", "add", "README.md", "vendor", "AGENTS.md"], cwd=tmp, check=True)

            def run():
                return subprocess.run([sys.executable, "-B", str(SCRIPT)],
                                      cwd=tmp, text=True, capture_output=True)

            result = run()
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("README.md:1: missing tracked target", result.stderr)
            self.assertNotIn("gone.md", result.stderr)
            subprocess.run(["git", "add", "new.md"], cwd=tmp, check=True)
            result = run()
            self.assertEqual(result.returncode, 0, result.stderr)
            (root / "new.md").unlink()
            result = run()
            self.assertEqual(result.returncode, 1)
            self.assertIn("tracked Markdown source is missing", result.stderr)


if __name__ == "__main__":
    unittest.main()
