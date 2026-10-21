#!/usr/bin/env python3
"""Draw the golden frames from the base and from the working tree, and say what moved.

    tools/golden-diff.py                 # against `git merge-base origin/main HEAD`
    tools/golden-diff.py --base HEAD^1   # what CI runs: a PR's merge commit against main
    tools/golden-diff.py --strict        # exit 1 if any frame moved: a refactor's proof

Both sides are drawn on this machine in this run, so the comparison is
byte-exact and needs no record: the base is `git archive`d into
`target/golden-base/src` and built against this checkout's own target
directory, which shares every registry dependency (about 20 s for the base and
17 s for the head on an M5 Pro). Both draw through source-compiled shaders, so a
stale Metal corpus on a shader PR cannot pass for a picture change.

Writes `target/golden-diff/`: `base/` and `head/` (the frames), `sheets/` (a
contact sheet per moved frame), `report.md` and `changed`, which Full CI's
summary job reads to require a `Golden frames moved:` line in the PR body.
When anything moved the head is drawn twice, and a frame that differs between
the two is reported as unstable rather than moved. That second draw exists to
measure nondeterminism the old committed record never separated from a change;
drop it once a few dozen PRs have reported none.

Run it under `./session-lifecycle.sh run --` like any build.
"""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
TARGET = ROOT / "target"
OUT = TARGET / "golden-diff"
BASE_SRC = TARGET / "golden-base" / "src"
TESTS = ["cargo", "test", "--no-fail-fast", "-p", "harmonigraph-render",
         "-p", "harmonigraph-offline", "golden"]

# A change touching only these cannot move a pixel. An exclusion list rather
# than a list of what draws, so that anything not named here — a toolchain pin,
# a cargo config, an asset — runs the comparison instead of skipping it.
NOT_DRAWN_DIRS = ("docs/", ".github/", ".claude/", ".agents/", ".githooks/", "tools/")
NOT_DRAWN_SUFFIXES = (".md",)


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, check=True, text=True,
                          stdout=subprocess.PIPE).stdout.strip()


def changed_files(base):
    tracked = git("diff", "--name-only", base).splitlines()
    untracked = git("ls-files", "--others", "--exclude-standard").splitlines()
    return [f for f in tracked + untracked if f]


def draws(path):
    return not (path.startswith(NOT_DRAWN_DIRS) or path.endswith(NOT_DRAWN_SUFFIXES))


def draw(tree, into):
    """Run the golden tests in `tree`, writing every frame under `into`."""
    environment = dict(os.environ, HARMONIGRAPH_GOLDEN_WRITE=str(into),
                       HARMONIGRAPH_REQUIRE_GPU="1", HARMONIGRAPH_SHADER_ASSETS="source",
                       CARGO_TARGET_DIR=str(TARGET))
    print(f"▶ drawing {tree.relative_to(ROOT) if tree != ROOT else 'the working tree'}"
          f" into {into.relative_to(ROOT)}", flush=True)
    return subprocess.run(TESTS, cwd=tree, env=environment).returncode


def frame_count(directory):
    return sum(1 for _ in directory.glob("*/*.png"))


def finish(report, changed):
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "report.md").write_text(report)
    (OUT / "changed").write_text("true" if changed else "false")
    print(report, end="")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--base", help="the ref to draw against (default: merge-base with origin/main)")
    parser.add_argument("--strict", action="store_true", help="exit 1 if any frame moved")
    args = parser.parse_args()

    base = git("rev-parse", args.base) if args.base else git("merge-base", "origin/main", "HEAD")
    shutil.rmtree(OUT, ignore_errors=True)

    if not any(draws(f) for f in changed_files(base)):
        finish(f"Golden frames not drawn: nothing that draws changed since {base[:10]}.\n", False)
        return 0

    # Reused while the base is unchanged, so its build stays incremental.
    stamp = BASE_SRC / ".golden-base"
    if not (stamp.is_file() and stamp.read_text() == base):
        shutil.rmtree(BASE_SRC, ignore_errors=True)
        BASE_SRC.mkdir(parents=True)
        archive = subprocess.Popen(["git", "archive", base], cwd=ROOT, stdout=subprocess.PIPE)
        subprocess.run(["tar", "-x", "-C", str(BASE_SRC)], stdin=archive.stdout, check=True)
        if archive.wait() != 0:
            sys.exit(f"git archive {base} failed")
        stamp.write_text(base)

    if draw(BASE_SRC, OUT / "base") != 0:
        print(f"⚠ the base's golden tests failed at {base[:10]}; comparing what it drew", flush=True)
    if frame_count(OUT / "base") == 0:
        finish(f"Golden frames not compared: the base {base[:10]} writes no frames "
               "(it predates tools/golden-diff.py).\n", False)
        return 0
    if draw(ROOT, OUT / "head") != 0:
        sys.exit("✗ the working tree's golden tests failed; the frames they did not draw "
                 "would read as removed, so there is no comparison to make")

    def compare(again):
        command = ["cargo", "run", "-q", "-p", "harmonigraph-golden", "--bin", "golden-compare",
                   "--", str(OUT / "base"), str(OUT / "head"), again, str(OUT)]
        return subprocess.run(command, cwd=ROOT, check=True, text=True,
                              stdout=subprocess.PIPE).stdout

    report = compare("-")
    if (OUT / "changed").read_text() == "true":
        if draw(ROOT, OUT / "again") != 0:
            sys.exit("✗ the working tree's golden tests failed on their second draw")
        report = compare(str(OUT / "again"))
    print(report, end="")

    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(f"## Golden frames against {base[:10]}\n\n{report}\n")
    changed = (OUT / "changed").read_text() == "true"
    unstable = "Unstable" in report
    return 1 if args.strict and (changed or unstable) else 0


if __name__ == "__main__":
    sys.exit(main())
