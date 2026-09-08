#!/usr/bin/env python3
"""Regression coverage for --check/--write agreement; all writes stay in tmp."""

import runpy
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "semantic-breaks.py"
semantic = runpy.run_path(str(SCRIPT))
reflow = semantic["reflow"]
offenders = semantic["offenders"]


class SemanticBreaksTests(unittest.TestCase):
    def assert_valid_unchanged(self, lines):
        self.assertEqual(offenders("fixture.md", lines), [])
        self.assertEqual(reflow(lines), lines)

    def assert_repaired(self, before, after):
        self.assertTrue(offenders("fixture.md", before))
        self.assertEqual(reflow(before), after)
        self.assert_valid_unchanged(after)

    def test_valid_list_continuations_keep_their_structure(self):
        self.assert_valid_unchanged([
            "- **Range.**",
            "  The lower endpoint stays here.",
            "  The upper endpoint stays there.",
            "",
            "  A second paragraph stays in the same item.",
            "  - A nested bullet.",
            "    Its continuation stays nested.",
            "  Back in the parent item.",
            "1. An ordered item.",
            "   Its continuation stays indented.",
            "10. A wider marker.",
            "    Its continuation stays indented too.",
            "2) Another ordered marker",
            "   with a continuation.",
            "",
        ])

    def test_accepted_clause_boundaries_are_not_normalized(self):
        self.assert_valid_unchanged([
            "The first clause ends here,",
            "and the second follows.",
            "There were 2.",
            "Both remain separate lines.",
            "Two sentences. Still one accepted line; with another clause.",
            "An accepted colon:",
            "followed by its own line.",
            "",
        ])

    def test_broken_list_prose_is_repaired_at_its_original_indent(self):
        self.assert_repaired([
            "- **Range.**",
            "  The lower endpoint",
            "  stays here. The upper endpoint",
            "  stays there.",
            "  This accepted clause stays separate,",
            "  as does this one.",
            "1. Ordered.",
            "   See [the guide. More details](https://example.test) and",
            "   `one. two` for details.",
            "",
        ], [
            "- **Range.**",
            "  The lower endpoint stays here. The upper endpoint stays there.",
            "  This accepted clause stays separate,",
            "  as does this one.",
            "1. Ordered.",
            "   See [the guide. More details](https://example.test) and `one. two` for details.",
            "",
        ])

    def test_broken_prose_is_repaired_without_losing_hard_breaks(self):
        self.assert_repaired([
            "An explicit hard break  ",
            "The broken",
            "sentence ends here; another",
            "clause ends with a hard break  ",
            "The final",
            "sentence ends here.",
            "",
        ], [
            "An explicit hard break  ",
            "The broken sentence ends here; another clause ends with a hard break  ",
            "The final sentence ends here.",
            "",
        ])

    def test_multiline_inline_spans_keep_their_structure_across_accepted_breaks(self):
        for opening, closing in [("`", "`"), ("[", "](https://example.test)")]:
            with self.subTest(opening=opening):
                self.assert_repaired([
                    f"Use {opening}these settings:",
                    "alpha. # Heading",
                    f"continues{closing} here.",
                    "",
                ], [
                    f"Use {opening}these settings:",
                    f"alpha. # Heading continues{closing} here.",
                    "",
                ])

    def test_structure_and_indent_changes_bound_repairs(self):
        structures = [
            ["# Heading"], ["- Bullet"], ["1. Ordered"], ["1) Ordered"],
            ["> A quote", "> continuation"], ["| A | B |"],
            ["<div>", "</div>"], ["[ref]: https://example.test"],
            ["    indented code", "    more code"], ["---"],
            ["```text", "not prose", "nor this", "```"],
            ["~~~~", "```", "not prose", "~~~", "still code", "~~~~"],
        ]
        for structure in structures:
            with self.subTest(structure=structure):
                self.assert_repaired(
                    ["Broken", "prose before"] + structure + ["Broken", "prose after"],
                    ["Broken prose before"] + structure + ["Broken prose after"],
                )
        # Indentation can switch list depth or introduce a lazy continuation;
        # do not move text across that boundary to normalize it.
        self.assert_valid_unchanged(["- Item", "  Indented", "lazy continuation", ""])
        self.assert_valid_unchanged(["---", "title: data", "not prose", "---", ""])
        self.assert_valid_unchanged(["---", "unclosed frontmatter", "still data"])
        self.assert_valid_unchanged(["~~~", "unclosed fence", "still code"])

    def test_cli_repairs_only_invalid_files_and_second_write_is_empty(self):
        valid = "- **Range.**\n  The lower endpoint stays here.\n  The upper endpoint stays there.\n"
        broken = "- **Range.**\n  The lower endpoint\n  stays here.\n"
        fixed = "- **Range.**\n  The lower endpoint stays here.\n"
        with tempfile.TemporaryDirectory(prefix="semantic-breaks-") as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-q", tmp], check=True)
            (root / "valid.md").write_text(valid)
            (root / "broken.md").write_text(broken)
            subprocess.run(["git", "add", "valid.md", "broken.md"], cwd=tmp, check=True)

            def run(mode):
                return subprocess.run(
                    [sys.executable, "-B", str(SCRIPT), mode],
                    cwd=tmp, text=True, capture_output=True,
                )

            check = run("--check")
            self.assertEqual(check.returncode, 1, check.stderr)
            self.assertIn("broken.md:2:", check.stderr)
            write = run("--write")
            self.assertEqual(write.returncode, 0, write.stderr)
            self.assertEqual(write.stdout, "  rewrote broken.md\n")
            self.assertEqual((root / "valid.md").read_text(), valid)
            self.assertEqual((root / "broken.md").read_text(), fixed)
            check = run("--check")
            self.assertEqual(check.returncode, 0, check.stderr)
            write = run("--write")
            self.assertEqual(write.returncode, 0, write.stderr)
            self.assertEqual(write.stdout, "")


if __name__ == "__main__":
    unittest.main()
