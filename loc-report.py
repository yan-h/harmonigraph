#!/usr/bin/env python3
"""Line-count report for this repo: totals by language, Rust's doc-comment
share, how much of Rust is test code, and per-crate size.

    ./loc-report.py                # human-readable report
    ./loc-report.py --json         # same numbers as one JSON object

Requires `tokei` on PATH (`brew install tokei`). tokei parses a doc comment
(`///`, `//!`) as an embedded Markdown "sub-language" of the Rust file it
lives in, so a language's own comment count excludes doc comments — they
only show up under that language's `children["Markdown"]`. Everything below
that recombines the two is doing so on purpose, not double-counting.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

EXCLUDES = ["target", "vendor", "__pycache__", ".claude/worktrees"]


def repo_root() -> Path:
    # -C anchors on the script's own location, not the caller's cwd, so
    # `./loc-report.py` from anywhere still measures the repo it lives in.
    out = subprocess.run(
        ["git", "-C", str(Path(__file__).resolve().parent), "rev-parse", "--show-toplevel"],
        capture_output=True, text=True, check=True,
    )
    return Path(out.stdout.strip())


def run_tokei(root: Path) -> dict:
    cmd = ["tokei", "--output", "json"]
    for e in EXCLUDES:
        cmd += ["--exclude", e]
    cmd.append(str(root))
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, check=True)
    except FileNotFoundError:
        sys.exit("tokei not found on PATH — `brew install tokei`")
    return json.loads(out.stdout)


def rust_doc_comment_split(tokei_json: dict) -> dict:
    rust = tokei_json.get("Rust", {})
    doc_reports = rust.get("children", {}).get("Markdown", [])
    doc = {
        "code": sum(r["stats"]["code"] for r in doc_reports),
        "comments": sum(r["stats"]["comments"] for r in doc_reports),
        "blanks": sum(r["stats"]["blanks"] for r in doc_reports),
    }
    plain = {k: rust.get(k, 0) for k in ("code", "comments", "blanks")}
    return {"plain": plain, "doc": doc}


def is_dedicated_test_file(rel_path: Path) -> bool:
    return "tests" in rel_path.parts[:-1] or rel_path.name == "tests.rs"


def cfg_test_block_lines(lines: list) -> int:
    """Sum the lines each `#[cfg(test)]` item spans, matching braces from
    its first `{` — covers both a `mod tests { ... }` and a single
    `#[cfg(test)] fn ...` item."""
    total = 0
    i, n = 0, len(lines)
    while i < n:
        if "#[cfg(test)]" not in lines[i]:
            i += 1
            continue
        j = i
        while j < n and "{" not in lines[j]:
            j += 1
        if j >= n:
            break
        depth = lines[j].count("{") - lines[j].count("}")
        k = j + 1
        while k < n and depth > 0:
            depth += lines[k].count("{") - lines[k].count("}")
            k += 1
        total += k - i
        i = k
    return total


def test_code_lines(root: Path) -> dict:
    dedicated, inline = 0, 0
    dedicated_files, inline_files = 0, 0
    for path in root.rglob("*.rs"):
        rel = path.relative_to(root)
        if any(rel.parts[0] == e.split("/")[0] for e in EXCLUDES):
            continue
        lines = path.read_text(errors="ignore").splitlines()
        if is_dedicated_test_file(rel):
            dedicated += len(lines)
            dedicated_files += 1
        else:
            n = cfg_test_block_lines(lines)
            if n:
                inline += n
                inline_files += 1
    return {
        "dedicated_lines": dedicated, "dedicated_files": dedicated_files,
        "inline_lines": inline, "inline_files": inline_files,
    }


def per_crate_lines(root: Path) -> dict:
    sizes = {}
    crates_dir = root / "crates"
    if crates_dir.is_dir():
        for crate in sorted(crates_dir.iterdir()):
            if not crate.is_dir():
                continue
            n = sum(len(p.read_text(errors="ignore").splitlines())
                    for p in crate.rglob("*.rs"))
            if n:
                sizes[crate.name] = n
    xtask = root / "xtask"
    if xtask.is_dir():
        n = sum(len(p.read_text(errors="ignore").splitlines())
                for p in xtask.rglob("*.rs"))
        if n:
            sizes["xtask"] = n
    return sizes


def build_report(root: Path) -> dict:
    tokei_json = run_tokei(root)
    languages = {
        k: {kk: v[kk] for kk in ("code", "comments", "blanks")}
        for k, v in tokei_json.items() if k != "Total"
    }
    total = {kk: tokei_json["Total"][kk] for kk in ("code", "comments", "blanks")}
    return {
        "languages": languages,
        "total": total,
        "rust_comments": rust_doc_comment_split(tokei_json),
        "test_code": test_code_lines(root),
        "per_crate": per_crate_lines(root),
    }


def print_report(r: dict) -> None:
    def row(label, d):
        lines = d["code"] + d["comments"] + d["blanks"]
        print(f"  {label:<24} {lines:>9,} {d['code']:>9,} {d['comments']:>9,} {d['blanks']:>9,}")

    print(f"{'Language':<26}{'Lines':>10}{'Code':>10}{'Comments':>10}{'Blank':>10}")
    for name, d in sorted(r["languages"].items(), key=lambda kv: -(kv[1]["code"] + kv[1]["comments"] + kv[1]["blanks"])):
        row(name, d)
    row("Total", r["total"])

    plain, doc = r["rust_comments"]["plain"], r["rust_comments"]["doc"]
    rust_lines = sum(plain.values()) + sum(doc.values())
    non_blank = rust_lines - plain["blanks"] - doc["blanks"]
    all_comments = plain["comments"] + doc["comments"]
    print()
    print(f"Rust comment density: {all_comments:,}/{non_blank:,} non-blank lines "
          f"= {100 * all_comments / non_blank:.1f}% comments "
          f"({doc['comments']:,} of those, {100 * doc['comments'] / non_blank:.1f}%, are doc comments)")

    tc = r["test_code"]
    test_total = tc["dedicated_lines"] + tc["inline_lines"]
    print()
    print(f"Test code: {test_total:,}/{rust_lines:,} Rust lines ({100 * test_total / rust_lines:.1f}%)")
    print(f"  dedicated tests/ files: {tc['dedicated_lines']:,} lines across {tc['dedicated_files']} files")
    print(f"  inline #[cfg(test)] blocks: {tc['inline_lines']:,} lines across {tc['inline_files']} files")

    print()
    print("Per-crate size (raw .rs line count):")
    for name, n in sorted(r["per_crate"].items(), key=lambda kv: -kv[1]):
        print(f"  {name:<26} {n:>9,}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--json", action="store_true", help="print the raw numbers as one JSON object")
    args = ap.parse_args()

    root = repo_root()
    report = build_report(root)
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print_report(report)


if __name__ == "__main__":
    main()
