#!/usr/bin/env python3
"""Lay out markdown prose one clause per line, and check that it stays that way.

`--check` is the CI mode: it reports lines that break mid-clause and exits 1.
`--write` rewrites the files. Both operate on the tracked `.md` files this repo
owns; `vendor/` is excluded so a local fork keeps diffing cleanly against
upstream.

A line break inside a paragraph renders as a space, so where the breaks fall
changes no rendered output. What it changes is the diff: an edit touches the
clause it edits instead of reflowing everything after it, two sessions editing
neighbouring sentences do not collide, and a line is a whole unit of text, so
splicing paragraphs cannot leave a fragment stranded on its own line.

Structure is left alone entirely — frontmatter, fenced code, tables, headings,
HTML blocks, indented code, and reference definitions are copied through
untouched. Inside prose, a break is only ever taken at a boundary that already
carries punctuation, and never inside a code span, a link target, or after an
abbreviation.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys

# Sentence enders, then the clause boundaries the spec allows. Breaking only
# after punctuation is what makes the check below a mechanical test rather than
# a judgement: every line of a paragraph but its last ends at one of these.
SENTENCE_END = re.compile(r"(?<=[.!?])\s+")
CLAUSE_END = re.compile(r"(?<=[;:—])\s+")

# A period that does NOT end a sentence. Version numbers and the Latin
# abbreviations this tree actually uses; a break after one reads as a sentence
# boundary that is not there.
ABBREV = re.compile(r"(?:^|\s)(?:e\.g|i\.e|cf|vs|etc|Dr|Mr|Ms|St|approx|Fig|no)\.$|\d\.$", re.I)

# Structure that is copied through rather than reflowed.
STRUCTURAL = re.compile(
    r"""^(
      \s*(\#{1,6})\s        # heading
    | \s*[-*+]\s            # bullet
    | \s*\d+\.\s            # ordered item
    | \s*>                  # block quote
    | \s*\|                 # table row
    | \s*<                  # html block
    | \s*\[[^\]]+\]:\s      # link reference definition
    | \s{4,}\S              # indented code
    | \s*(-{3,}|={3,}|\*{3,})\s*$   # rule / setext underline
    )""",
    re.X,
)


def _spans_to_protect(text: str) -> list[tuple[int, int]]:
    """Character ranges a break must not fall inside."""
    spans: list[tuple[int, int]] = []
    for m in re.finditer(r"`[^`]*`", text):  # inline code
        spans.append(m.span())
    for m in re.finditer(r"\[[^\]]*\]\([^)]*\)", text):  # inline link
        spans.append(m.span())
    return spans


def _inside(pos: int, spans: list[tuple[int, int]]) -> bool:
    return any(a < pos < b for a, b in spans)


def split_clauses(text: str) -> list[str]:
    """Break `text` after sentences, then after clause punctuation."""
    protect = _spans_to_protect(text)

    def cut(chunk: str, pattern: re.Pattern[str], offset: int) -> list[str]:
        out, last = [], 0
        for m in pattern.finditer(chunk):
            if _inside(offset + m.start(), protect):
                continue
            head = chunk[last : m.start()]
            if ABBREV.search(head):
                continue
            out.append(head)
            last = m.end()
        out.append(chunk[last:])
        return [p for p in out if p.strip()]

    pieces, base = [], 0
    for sentence in cut(text, SENTENCE_END, 0):
        start = text.index(sentence, base)
        base = start + len(sentence)
        pieces.extend(cut(sentence, CLAUSE_END, start))
    return [p.strip() for p in pieces if p.strip()]


def reflow(lines: list[str]) -> list[str]:
    """Rewrite prose paragraphs one clause per line; copy structure through."""
    out: list[str] = []
    fence: str | None = None
    para: list[str] = []
    i = 0

    # YAML frontmatter is data, not prose.
    if lines and lines[0].strip() == "---":
        for j in range(1, len(lines)):
            if lines[j].strip() == "---":
                out.extend(lines[: j + 1])
                i = j + 1
                break

    def flush() -> None:
        if not para:
            return
        # A trailing double space is a hard break and is load-bearing; leave the
        # whole paragraph alone rather than guess where it wanted to break.
        if any(p.endswith("  ") for p in para[:-1]):
            out.extend(para)
        else:
            out.extend(split_clauses(" ".join(p.strip() for p in para)))
        para.clear()

    while i < len(lines):
        line = lines[i]
        stripped = line.strip()
        if stripped.startswith("```") or stripped.startswith("~~~"):
            flush()
            marker = stripped[:3]
            fence = None if fence == marker else marker
            out.append(line)
        elif fence is not None:
            out.append(line)
        elif not stripped or STRUCTURAL.match(line):
            flush()
            out.append(line)
        else:
            para.append(line)
        i += 1
    flush()
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
        if stripped.startswith("```") or stripped.startswith("~~~"):
            marker = stripped[:3]
            fence = None if fence == marker else marker
            continue
        if fence is not None or not stripped or STRUCTURAL.match(line):
            continue
        nxt = lines[n] if n < len(lines) else ""
        if not nxt.strip() or STRUCTURAL.match(nxt):
            continue  # last line of a paragraph may end anywhere
        if line.endswith("  "):
            continue  # explicit hard break
        if not re.search(r"[.!?;:—,]$", stripped):
            bad.append((n, stripped))
    return bad


def tracked() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "*.md"], capture_output=True, text=True, check=True
    ).stdout.split("\n")
    # AGENTS.md and GEMINI.md are symlinks to CLAUDE.md; following them would
    # rewrite one file three times and report it three times.
    return [
        f
        for f in out
        if f and not f.startswith("vendor/") and not os.path.islink(f)
    ]


def main() -> int:
    check = "--check" in sys.argv
    write = "--write" in sys.argv
    if check == write:
        print("usage: semantic-breaks.py --check | --write", file=sys.stderr)
        return 2

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
