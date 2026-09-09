#!/usr/bin/env python3
"""Lay out markdown prose one clause per line, and check that it stays that way.

`--check` is the CI mode: it reports lines that break mid-clause and exits 1.
`--write` repairs those breaks, leaving accepted lines alone. Both operate on
the tracked `.md` files this repo owns; `vendor/` is excluded so a local fork
keeps diffing cleanly against upstream.
Untracked Markdown is skipped with a warning: stage new documents with
`git add` before checking or repairing them. Ignored files are not considered.

A line break inside a paragraph renders as a space, so where the breaks fall
changes no rendered output. What it changes is the diff: an edit touches the
clause it edits instead of reflowing everything after it, two sessions editing
neighbouring sentences do not collide, and a line is a whole unit of text, so
splicing paragraphs cannot leave a fragment stranded on its own line.

Structure is left alone entirely — frontmatter, fenced code, tables, headings,
HTML blocks, indented code, and reference definitions are copied through
untouched. Inside prose, repairs join mid-clause breaks without introducing
new ones. Accepted boundaries stay in place, including those inside multiline
code spans and links.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys

# Structure that is copied through rather than reflowed.
STRUCTURAL = re.compile(
    r"""^(
      \s*(\#{1,6})\s        # heading
    | \s*[-*+]\s            # bullet
    | \s*\d+[.)]\s          # ordered item
    | \s*>                  # block quote
    | \s*\|                 # table row
    | \s*<                  # html block
    | \s*\[[^\]]+\]:\s      # link reference definition
    | \s{4,}\S              # indented code
    | \s*(-{3,}|={3,}|\*{3,})\s*$   # rule / setext underline
    )""",
    re.X,
)
FENCE = re.compile(r"^(`{3,}|~{3,})")


def _indent(line: str) -> str:
    return line[: len(line) - len(line.lstrip())]


def reflow(lines: list[str]) -> list[str]:
    """Repair only reported breaks, retaining indentation and valid boundaries."""
    # Share the checker's structural decisions: a passing file is a no-op,
    # including comma breaks and optional clause splits the checker accepts.
    bad = {n for n, _ in offenders("", lines)}
    out: list[str] = []
    i = 0
    while i < len(lines):
        start = i
        while i + 1 in bad:
            i += 1
        if start == i:
            out.append(lines[i])
        else:
            # The checker never crosses an indentation change. Preserve that
            # prefix so list continuations stay in place. Only join: inserting
            # new breaks could split an inline span opened on an accepted line
            # before this run, turning its text into a heading or list marker.
            indent = _indent(lines[start])
            joined = " ".join(p.strip() for p in lines[start : i + 1])
            # The last line may end in an explicit Markdown hard break.
            suffix = lines[i][len(lines[i].rstrip()) :]
            out.append(indent + joined + suffix)
        i += 1
    return out


def offenders(path: str, lines: list[str]) -> list[tuple[int, str]]:
    """Prose lines that break mid-clause with more prose after them."""
    bad: list[tuple[int, str]] = []
    fence: str | None = None
    in_front = bool(lines) and lines[0].strip() == "---"
    for n, line in enumerate(lines, 1):
        stripped = line.strip()
        if in_front:
            if n > 1 and stripped == "---":
                in_front = False
            continue
        if fence is not None:
            if stripped.startswith(fence) and not stripped.strip(fence[0]):
                fence = None
            continue
        marker = FENCE.match(stripped)
        if marker:
            fence = marker[0]
            continue
        if not stripped or STRUCTURAL.match(line):
            continue
        nxt = lines[n] if n < len(lines) else ""
        if not nxt.strip() or STRUCTURAL.match(nxt) or FENCE.match(nxt.strip()):
            continue  # last line of a paragraph may end anywhere
        if _indent(line) != _indent(nxt):
            continue  # preserve list depth and lazy continuation boundaries
        if line.endswith("  "):
            continue  # explicit hard break
        if not re.search(r"[.!?;:—,]$", stripped):
            bad.append((n, stripped))
    return bad


def markdown_files(*options: str) -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z", *options, "--", "*.md"],
        capture_output=True, text=True, check=True,
    ).stdout.split("\0")
    # AGENTS.md and GEMINI.md are symlinks to CLAUDE.md; following them would
    # rewrite one file three times and report it three times.
    return [
        f
        for f in out
        if f and not f.startswith("vendor/") and not os.path.islink(f)
    ]


def tracked() -> list[str]:
    return markdown_files()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="report mid-clause breaks")
    mode.add_argument("--write", action="store_true", help="repair only reported breaks")
    args = parser.parse_args()
    check, write = args.check, args.write

    for path in markdown_files("--others", "--exclude-standard"):
        print(
            f"warning: untracked Markdown skipped: {path}; stage with git add before validation",
            file=sys.stderr,
        )

    failures = 0
    for path in tracked():
        with open(path, encoding="utf-8") as fh:
            lines = fh.read().split("\n")
        if write:
            new = reflow(lines)
            if new != lines:
                with open(path, "w", encoding="utf-8") as fh:
                    fh.write("\n".join(new))
                print(f"  rewrote {path}")
        else:
            for n, text in offenders(path, lines):
                print(f"{path}:{n}: line breaks mid-clause: {text[-60:]!r}", file=sys.stderr)
                failures += 1

    if check and failures:
        print(
            f"\n{failures} prose line(s) break mid-clause. Prose is laid out one\n"
            "clause per line so an edit touches one line. Run\n"
            "  .claude/semantic-breaks.py --write\n",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
