#!/usr/bin/env python3
"""Check local Markdown links without network access or third-party packages.

Reads tracked, non-vendor .md files, skipping duplicate symlink sources.
Inline links/images and single-line reference definitions must name a tracked
file or directory. Fragments on Markdown files must name a heading (GitHub's
lowercase, punctuation-stripped IDs with duplicate suffixes). Other file types
are checked for existence only. Web URLs and absolute paths are not checked.
Fenced/indented code, inline code examples and HTML comments are ignored.
This is a repository link gate, not a full Markdown renderer: raw HTML links
and custom HTML anchors are outside its scope. Stage new documents and assets
with git add before running it, just like semantic-breaks.py.
"""

from __future__ import annotations

import argparse
import html
import posixpath
import re
import string
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit

FENCE = re.compile(r"^\s*(`{3,}|~{3,})")
INLINE = re.compile(r"(?<!\\)!?\[((?:\\.|[^\[\]\\]|\[[^\]]*\])*)\]\(\s*")
REFERENCE = re.compile(r"^ {0,3}\[[^\]]+\]:[ \t]*", re.M)
ESCAPE = re.compile(r"\\([" + re.escape(string.punctuation) + r"])")


def blank(text: str) -> str:
    return re.sub(r"[^\n]", " ", text)


def prose(text: str) -> str:
    """Mask examples without moving line numbers or heading boundaries."""
    # Commented-out examples must not open a fence over the live prose after them.
    text = re.sub(r"<!--.*?(?:-->|\Z)", lambda m: blank(m[0]), text, flags=re.S)
    lines = text.splitlines(keepends=True)
    fence = None
    frontmatter = bool(lines) and lines[0].strip() == "---"
    for n, line in enumerate(lines):
        if frontmatter:
            if n > 0 and line.strip() == "---":
                frontmatter = False
            lines[n] = blank(line)
            continue
        if fence:
            if re.fullmatch(r"\s*" + re.escape(fence[0]) + "{" + str(len(fence)) + r",}\s*", line):
                fence = None
            lines[n] = blank(line)
            continue
        marker = FENCE.match(line)
        if marker:
            fence = marker[1]
            lines[n] = blank(line)
        elif line.startswith(("    ", "\t")):
            lines[n] = blank(line)
    return "".join(lines)


def destination(text: str, start: int) -> str:
    """Read an angle destination or a bare URL with balanced/escaped parens."""
    angle = text[start:start + 1] == "<"
    if angle:
        start += 1
    n, depth = start, 0
    while n < len(text):
        char = text[n]
        if char == "\\" and n + 1 < len(text):
            n += 2
            continue
        if angle:
            if char == ">":
                break
        elif char.isspace() or (char == ")" and depth == 0):
            break
        elif char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
        n += 1
    return ESCAPE.sub(r"\1", html.unescape(text[start:n]))


def links(text: str):
    text = re.sub(
        r"(?<!`)(`+)(?!`)(.*?)\1(?!`)", lambda m: blank(m[0]), text, flags=re.S,
    )
    for pattern in (INLINE, REFERENCE):
        for match in pattern.finditer(text):
            yield text.count("\n", 0, match.start()) + 1, destination(text, match.end())


def anchors(text: str) -> set[str]:
    result = set()
    previous = ""
    for line in text.splitlines():
        match = re.match(r"^ {0,3}#{1,6}(?:[ \t]+|$)(.*)", line)
        if match:
            heading = re.sub(r"[ \t]+#+[ \t]*$", "", match[1])
        elif previous.strip() and re.fullmatch(r" {0,3}(?:=+|-+)[ \t]*", line):
            heading = previous.strip()
        else:
            previous = line
            continue
        # Heading links contribute their label, and inline code its contents.
        heading = re.sub(r"!?\[([^]]*)\]\([^)]*\)", r"\1", heading)
        heading = re.sub(r"<[^>]*>", "", heading)
        slug = re.sub(r"[^\w\s-]", "", html.unescape(heading).lower()).replace(" ", "-")
        candidate, suffix = slug, 0
        while candidate in result:
            suffix += 1
            candidate = f"{slug}-{suffix}"
        result.add(candidate)
        previous = ""
    return result


def check(root: Path, tracked: set[str]) -> list[str]:
    sources = sorted(p for p in tracked if p.endswith(".md")
                     and not p.startswith("vendor/") and not (root / p).is_symlink())
    heading_ids: dict[str, set[str]] = {}
    failures = []
    for source in sources:
        if not (root / source).is_file():
            failures.append(f"{source}: tracked Markdown source is missing")
            continue
        text = prose((root / source).read_text(encoding="utf-8"))
        for line, url in links(text):
            if url.startswith("/") or re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:", url):
                continue
            parsed = urlsplit(url)
            target = posixpath.normpath(posixpath.join(posixpath.dirname(source), unquote(parsed.path))) if parsed.path else source
            path = root / target
            is_tracked = target == "." or target in tracked or any(p.startswith(target.rstrip("/") + "/") for p in tracked)
            if not is_tracked or not path.exists():
                failures.append(f"{source}:{line}: missing tracked target: {url!r} ({target})")
            elif parsed.fragment and target.endswith(".md"):
                if target not in heading_ids:
                    heading_ids[target] = anchors(prose(path.read_text(encoding="utf-8")))
                if unquote(parsed.fragment) not in heading_ids[target]:
                    failures.append(f"{source}:{line}: missing heading anchor: {url!r}")
    return failures


def main() -> int:
    argparse.ArgumentParser(description=__doc__).parse_args()
    tracked = set(subprocess.run(
        ["git", "ls-files", "-z"], capture_output=True, text=True, check=True,
    ).stdout.split("\0")) - {""}
    failures = check(Path.cwd(), tracked)
    for failure in failures:
        print(failure, file=sys.stderr)
    if failures:
        return 1
    print("✓ tracked Markdown local targets and heading anchors resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())
