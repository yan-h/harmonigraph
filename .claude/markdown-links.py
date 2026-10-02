#!/usr/bin/env python3
"""Check local Markdown links without network access or third-party packages.

Reads tracked, non-vendor .md files, skipping duplicate symlink sources.
Inline links/images and single-line reference definitions must name a tracked
file or directory. Fragments on Markdown files must name a heading (GitHub's
lowercase, punctuation-stripped IDs with duplicate suffixes). Other file types
are checked for existence only. Web URLs and absolute paths are not checked.
Links inside fenced/indented code, inline code and HTML comments are ignored.
This is a repository link gate, not a full Markdown renderer: raw HTML links
and custom HTML anchors are outside its scope. Stage new documents and assets
with git add before running it, just like semantic-breaks.py.

Inline code naming a repository path (`crates/x/src/y.rs`, `tools/z.py:12`)
must name a tracked file or directory too, because a moved or deleted file
leaves exactly that behind and the audits kept finding it by hand. Only paths
under a tracked top-level directory are checked; patterns, placeholders,
gitignored paths and docs/evidence (a frozen archive) are skipped, and so is a
code span that is a link's label: the link is what is checked, so a file that is
gone on purpose is cited as a link to a commit where it existed.
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
CODE_SPAN = re.compile(r"(?<!`)`([^`\n]+)`(?!`)")


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


def code_paths(text: str, tops: set[str]):
    """Repository paths named in inline code that is not a link's label."""
    for n, line in enumerate(text.splitlines(), 1):
        labels = [label.span(1) for label in INLINE.finditer(line)]
        for match in CODE_SPAN.finditer(line):
            if any(start <= match.start() and match.end() <= end for start, end in labels):
                continue
            # Drop a fragment (`#L3`, `#heading`) and any `:line`, `:line:col` or `:a-b`.
            path = re.sub(r"(?::\d+(?:-\d+)?)+$", "", match[1].strip().split("#")[0])
            path = path.removeprefix("./").rstrip("/")
            if (path.split("/")[0] in tops and "/" in path
                    and not re.search(r"[\s*?<>{}\[\]$]|\.\.\.|-$", path)):
                yield n, path


def check(root: Path, tracked: set[str], ignored=lambda path: False) -> list[str]:
    sources = sorted(p for p in tracked if p.endswith(".md")
                     and not p.startswith("vendor/") and not (root / p).is_symlink())
    heading_ids: dict[str, set[str]] = {}
    failures = []
    dirs = {"/".join(parts[:n]) for parts in (p.split("/") for p in tracked) for n in range(1, len(parts))}
    tops = {d for d in dirs if "/" not in d}
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
            is_tracked = target == "." or target in tracked or target.rstrip("/") in dirs
            if not is_tracked or not path.exists():
                failures.append(f"{source}:{line}: missing tracked target: {url!r} ({target})")
            elif parsed.fragment and target.endswith(".md"):
                if target not in heading_ids:
                    heading_ids[target] = anchors(prose(path.read_text(encoding="utf-8")))
                if unquote(parsed.fragment) not in heading_ids[target]:
                    failures.append(f"{source}:{line}: missing heading anchor: {url!r}")
        if source.startswith("docs/evidence/"):
            continue
        for line, path in code_paths(text, tops):
            if path not in tracked and path not in dirs and not ignored(path):
                failures.append(f"{source}:{line}: inline code names a missing tracked path: {path!r}"
                                " (if it is gone on purpose, link it to a commit where it existed)")
    return failures


def main() -> int:
    argparse.ArgumentParser(description=__doc__).parse_args()
    tracked = set(subprocess.run(
        ["git", "ls-files", "-z"], capture_output=True, text=True, check=True,
    ).stdout.split("\0")) - {""}
    def ignored(path: str) -> bool:
        # A directory-only pattern (`dir/`) matches only the slash form.
        return any(subprocess.run(["git", "check-ignore", "-q", "--no-index", name]).returncode == 0
                   for name in (path, path + "/"))

    failures = check(Path.cwd(), tracked, ignored)
    for failure in failures:
        print(failure, file=sys.stderr)
    if failures:
        return 1
    print("✓ tracked Markdown local targets, heading anchors and inline code paths resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())
