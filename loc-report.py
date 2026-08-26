#!/usr/bin/env python3
"""Line-count report for this repo: totals by language, Rust's doc-comment
share, how much of Rust is test code, and per-crate size — headlined by the
number that's neither: non-test, non-comment Rust code.

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
import tempfile
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


def run_tokei(*paths: Path) -> dict:
    cmd = ["tokei", "--output", "json"]
    for e in EXCLUDES:
        cmd += ["--exclude", e]
    cmd += [str(p) for p in paths]
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


def cfg_test_block_ranges(lines: list) -> list:
    """Line ranges (start, end-exclusive) each `#[cfg(test)]` item spans,
    matching braces from its first `{` — covers both a `mod tests { ... }`
    and a single `#[cfg(test)] fn ...` item."""
    ranges = []
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
        ranges.append((i, k))
        i = k
    return ranges


def crate_of(rel_path: Path) -> str:
    parts = rel_path.parts
    if parts[0] == "crates":
        return parts[1]
    if parts[0] == "xtask":
        return "xtask"
    return "other"


def code_lines_of(tokei_json: dict) -> int:
    return tokei_json.get("Rust", {}).get("code", 0)


def rust_breakdown(root: Path, reports: list) -> dict:
    """Per-file and per-crate split of Rust `code` lines (comments already
    excluded by tokei) into test vs. non-test, by combining each report's
    own `code` count with a fresh tokei pass over just its test lines —
    the only way to get a code-only count for an extracted line range."""
    totals = {"total_code": 0, "test_code": 0, "dedicated_files": 0, "inline_files": 0,
              "dedicated_lines": 0, "inline_lines": 0}
    per_crate = {}

    def bump(crate, key, amount):
        per_crate.setdefault(crate, {"total_code": 0, "test_code": 0})
        per_crate[crate][key] += amount

    with tempfile.TemporaryDirectory() as tmp:
        for report in reports:
            path = Path(report["name"])
            rel = path.relative_to(root)
            crate = crate_of(rel)
            code = report["stats"]["code"]
            totals["total_code"] += code
            bump(crate, "total_code", code)

            lines = path.read_text(errors="ignore").splitlines()
            if is_dedicated_test_file(rel):
                totals["test_code"] += code
                totals["dedicated_lines"] += len(lines)
                totals["dedicated_files"] += 1
                bump(crate, "test_code", code)
                continue

            ranges = cfg_test_block_ranges(lines)
            if not ranges:
                continue
            chunk = "\n".join(l for start, end in ranges for l in lines[start:end])
            tmp_file = Path(tmp) / (rel.as_posix().replace("/", "__") + ".rs")
            tmp_file.write_text(chunk)
            test_code = code_lines_of(run_tokei(tmp_file))
            totals["test_code"] += test_code
            totals["inline_lines"] += sum(end - start for start, end in ranges)
            totals["inline_files"] += 1
            bump(crate, "test_code", test_code)

    totals["prod_code"] = totals["total_code"] - totals["test_code"]
    for stats in per_crate.values():
        stats["prod_code"] = stats["total_code"] - stats["test_code"]
    return {"totals": totals, "per_crate": per_crate}


def build_report(root: Path) -> dict:
    tokei_json = run_tokei(root)
    rust_code = rust_breakdown(root, tokei_json.get("Rust", {}).get("reports", []))
    rb = rust_code["totals"]

    # Test/prod code is a Rust-only concept here (#[cfg(test)], tests/ dirs)
    # — every other language's code counts as prod code, none as test code.
    languages = {}
    for k, v in tokei_json.items():
        if k == "Total":
            continue
        d = {kk: v[kk] for kk in ("code", "comments", "blanks")}
        if k == "Rust":
            d["test_code"], d["prod_code"] = rb["test_code"], rb["prod_code"]
        else:
            d["test_code"], d["prod_code"] = 0, d["code"]
        languages[k] = d

    total = {kk: tokei_json["Total"][kk] for kk in ("code", "comments", "blanks")}
    total["test_code"] = rb["test_code"]
    total["prod_code"] = total["code"] - rb["test_code"]

    return {
        "languages": languages,
        "total": total,
        "rust_comments": rust_doc_comment_split(tokei_json),
        "rust_code": rust_code,
    }


def print_report(r: dict) -> None:
    rb = r["rust_code"]["totals"]
    pct = 100 * rb["prod_code"] / rb["total_code"]
    print(f">>> Non-test, non-comment Rust code: {rb['prod_code']:,} lines ({pct:.1f}% of all Rust code)")
    print()

    def row(label, d):
        lines = d["code"] + d["comments"] + d["blanks"]
        print(f"  {label:<24} {lines:>9,} {d['code']:>9,} {d['comments']:>9,} {d['blanks']:>9,}"
              f" {d['test_code']:>10,} {d['prod_code']:>10,}")

    print(f"{'Language':<26}{'Lines':>10}{'Code':>10}{'Comments':>10}{'Blank':>10}"
          f"{'Test code':>11}{'Prod code':>11}")
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

    test_total = rb["dedicated_lines"] + rb["inline_lines"]
    print()
    print(f"Test code: {test_total:,}/{rust_lines:,} Rust lines ({100 * test_total / rust_lines:.1f}%), "
          f"{rb['test_code']:,} of which are non-comment code lines")
    print(f"  dedicated tests/ files: {rb['dedicated_lines']:,} lines across {rb['dedicated_files']} files")
    print(f"  inline #[cfg(test)] blocks: {rb['inline_lines']:,} lines across {rb['inline_files']} files")

    print()
    print(f"{'Crate':<26}{'Code':>10}{'Test code':>12}{'Prod code':>12}")
    for name, d in sorted(r["rust_code"]["per_crate"].items(), key=lambda kv: -kv[1]["prod_code"]):
        print(f"  {name:<24} {d['total_code']:>9,} {d['test_code']:>11,} {d['prod_code']:>11,}")


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
