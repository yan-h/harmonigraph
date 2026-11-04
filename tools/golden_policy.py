#!/usr/bin/env python3
"""Require complete golden evidence and every changed row in the live PR body."""
import argparse
import os
from pathlib import Path
import subprocess


def check_golden(directory, body):
    report = (directory / "report.md").read_text()
    if not report.strip():
        raise ValueError("Golden report is empty")
    changed = (directory / "changed").read_text()
    missing = [row for row in changed.splitlines() if row and row not in body]
    if missing:
        raise ValueError("Golden rows missing from the PR body:\n" + "\n".join(missing))
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY"))
    parser.add_argument("--pr", default=os.environ.get("PR"))
    args = parser.parse_args()
    if not args.repo or not args.pr:
        parser.error("--repo and --pr are required outside GitHub Actions")
    body = subprocess.check_output(
        ["gh", "pr", "view", str(args.pr), "--repo", args.repo, "--json", "body", "--jq", ".body"],
        text=True,
    )
    print(check_golden(args.directory, body), end="")
    print("Every changed golden frame is stated in the live PR body.")


if __name__ == "__main__":
    main()
