#!/usr/bin/env python3
"""Draw the golden frames from the base and from the working tree, and say what moved.

    tools/golden-diff.py                 # against `git merge-base origin/main HEAD`
    tools/golden-diff.py --base HEAD^1   # what CI runs: a PR's merge commit against main
    tools/golden-diff.py --strict        # exit 1 if any frame moved: a refactor's proof
    tools/golden-diff.py --draw-only DIR # draw the working tree alone: what CI caches per main commit
    tools/golden-diff.py --base HEAD^1 --base-frames DIR  # against those, drawing the base only to confirm a move
    tools/golden-diff.py --frames-key REF  # what CI caches REF's frames under

A reported move is always between two draws on this machine in this run, so the
comparison is byte-exact and needs no record. (In CI an answer of no move may
instead come from frames cached on the same macOS build; see below.) The base
is `git archive`d into `target/golden-base/src` and built into
`target/golden-base/target`, a target directory of its own. Sharing this checkout's would save rebuilding the registry
dependencies, but cargo names a workspace crate's artifacts the same in both
trees, so whichever tree built a crate last is what the other links against
unless something path-dependent in the crate forces a rebuild: #1572's head
linked the base's `harmonigraph-analysis` and failed to compile, and a head that
had compiled would have been drawn with the base's code. sccache serves the
registry crates either way, and the base's directory persists with its source,
so only the first draw against a new base pays for it. Both draw through
source-compiled shaders, so a stale Metal corpus on a shader PR cannot pass for
a picture change.

Writes `target/golden-diff/`: `base/` and `head/` (the frames), `sheets/` (a
contact sheet per moved frame), `report.md`, and `changed`: the table's rows,
empty when nothing changed, each of which Full CI's summary job requires the PR
body to carry verbatim. `--strict` also fails when the base predates the tool,
since then nothing was compared.
When anything moved the head is drawn twice, and a frame that differs between
the two is reported as unstable rather than moved. That second draw exists to
measure nondeterminism the old committed record never separated from a change;
drop it once a few dozen PRs have reported none.

CI saves each main commit's frames (`--draw-only`), and a PR compares against
its base's (`--base-frames`) rather than building the base. Frames that report
no change are the answer. Frames that report one are only a lead: the base is
then drawn here and the comparison made again, so every reported move is still
between two draws on one machine in one run, and a cached set that differs
from what the base draws now can cost a base build but never hide a move or
invent one. A missing directory is a cache miss, and draws the base as before.
The cache key (`--frames-key`) is what decides the frames: the blobs of REF's
tree that `draws` counts, and this script, the recipe they are drawn by. So a
merge that touches only what does not draw shares its parent's frames, and a
PR that changes the recipe misses and draws its base here.

Run it under `./session-lifecycle.sh run --` like any build.
"""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
TARGET = ROOT / "target"
OUT = TARGET / "golden-diff"
BASE_SRC = TARGET / "golden-base" / "src"
BASE_TARGET = TARGET / "golden-base" / "target"
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
    # --no-renames: a file moved out of `crates/` into an excluded directory
    # lists its old path too, so the move still counts as a change that draws.
    tracked = git("diff", "--no-renames", "--name-only", base).splitlines()
    untracked = git("ls-files", "--others", "--exclude-standard").splitlines()
    return [f for f in tracked + untracked if f]


def draws(path):
    return not (path.startswith(NOT_DRAWN_DIRS) or path.endswith(NOT_DRAWN_SUFFIXES))


def frames_key(ref):
    """What decides the frames `ref` draws: every blob `draws` counts, and this recipe.

    The script is read from the working tree because it is the recipe this run
    draws by, whichever commit's frames it is keying."""
    digest = hashlib.sha256(Path(__file__).read_bytes())
    for entry in git("ls-tree", "-r", "-z", "--full-tree", ref).split("\0"):
        if entry and draws(entry.split("\t", 1)[1]):
            digest.update(entry.encode() + b"\0")
    return digest.hexdigest()[:40]


def draw(tree, into, target):
    """Run the golden tests in `tree`, building into `target`, writing every frame under `into`."""
    environment = dict(os.environ, HARMONIGRAPH_GOLDEN_WRITE=str(into),
                       HARMONIGRAPH_REQUIRE_GPU="1", HARMONIGRAPH_SHADER_ASSETS="source",
                       CARGO_TARGET_DIR=str(target))
    print(f"▶ drawing {tree.relative_to(ROOT) if tree != ROOT else 'the working tree'}"
          f" into {os.path.relpath(into, ROOT)}", flush=True)
    return subprocess.run(TESTS, cwd=tree, env=environment).returncode



def skip(report):
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "report.md").write_text(report)
    (OUT / "changed").write_text("")
    print(report, end="")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--base", help="the ref to draw against (default: merge-base with origin/main)")
    parser.add_argument("--strict", action="store_true", help="exit 1 if any frame moved")
    parser.add_argument("--base-frames", type=Path, metavar="DIR",
                        help="the base's frames, drawn earlier; the base is drawn only if they differ")
    parser.add_argument("--draw-only", type=Path, metavar="DIR",
                        help="draw the working tree into DIR, only if every frame draws, and stop")
    parser.add_argument("--frames-key", metavar="REF",
                        help="print the cache key for REF's frames, and stop")
    args = parser.parse_args()
    if args.frames_key:
        print(frames_key(args.frames_key))
        return 0
    # The tests run in their crates' directories, so what they write to is absolute.
    args.base_frames = args.base_frames and args.base_frames.resolve()
    args.draw_only = args.draw_only and args.draw_only.resolve()

    if args.draw_only:
        # Drawn aside and moved in whole, so a set missing a frame is never
        # there to be cached and later read as a base that lost it.
        partial = args.draw_only.with_name(args.draw_only.name + ".partial")
        shutil.rmtree(partial, ignore_errors=True)
        shutil.rmtree(args.draw_only, ignore_errors=True)
        if draw(ROOT, partial, TARGET) != 0:
            sys.exit("✗ the working tree's golden tests failed; nothing saved")
        if not partial.is_dir():
            sys.exit(f"✗ the golden tests passed but wrote no frame into {partial}; nothing saved")
        partial.rename(args.draw_only)
        return 0

    base = git("rev-parse", args.base) if args.base else git("merge-base", "origin/main", "HEAD")
    shutil.rmtree(OUT, ignore_errors=True)

    if not any(draws(f) for f in changed_files(base)):
        skip(f"Golden frames not drawn: nothing that draws changed since {base[:10]}.\n")
        return 0

    def compare(again):
        command = ["cargo", "run", "-q", "-p", "harmonigraph-golden", "--bin", "golden-compare",
                   "--", str(OUT / "base"), str(OUT / "head"), again, str(OUT)]
        return subprocess.run(command, cwd=ROOT, check=True, text=True,
                              stdout=subprocess.PIPE).stdout

    head_drawn = False
    if args.base_frames and args.base_frames.is_dir():
        # Copied rather than read in place so the artifact carries the base
        # this run compared against, whichever way it was had.
        shutil.copytree(args.base_frames, OUT / "base")
        if draw(ROOT, OUT / "head", TARGET) != 0:
            sys.exit("✗ the working tree's golden tests failed; the frames they did not draw "
                     "would read as removed, so there is no comparison to make")
        head_drawn = True
        report = compare("-")
        if not (OUT / "changed").read_text():
            return finish(base, report, args.strict)
        print(f"▶ the frames cached for {base[:10]} report a change; drawing the base to "
              "confirm it", flush=True)
        shutil.rmtree(OUT / "base")
    elif args.base_frames:
        print(f"▶ no frames cached for {base[:10]}; drawing the base", flush=True)

    # Reused while the base is unchanged, so its build stays incremental. A new
    # base is extracted with the time of extraction (`-m`) rather than each
    # file's commit time: cargo decides freshness by mtime, and a base committed
    # before the last base was built would otherwise read as older than that
    # build's artifacts and link them.
    stamp = BASE_SRC / ".golden-base"
    if not (stamp.is_file() and stamp.read_text() == base):
        shutil.rmtree(BASE_SRC, ignore_errors=True)
        BASE_SRC.mkdir(parents=True)
        archive = subprocess.Popen(["git", "archive", base], cwd=ROOT, stdout=subprocess.PIPE)
        subprocess.run(["tar", "-x", "-m", "-C", str(BASE_SRC)], stdin=archive.stdout, check=True)
        if archive.wait() != 0:
            sys.exit(f"git archive {base} failed")
        stamp.write_text(base)

    # Only a base whose gate cannot write frames is skipped. One that can and
    # fails to draw some is compared for what it did draw: each frame it did
    # not reads as new, so it has to be stated like any change, and a broken
    # main cannot block the PR that fixes it.
    gate = BASE_SRC / "crates/harmonigraph-golden/src/lib.rs"
    if not gate.is_file() or "HARMONIGRAPH_GOLDEN_WRITE" not in gate.read_text():
        skip(f"Golden frames not compared: the base {base[:10]} predates tools/golden-diff.py.\n")
        return 1 if args.strict else 0
    if draw(BASE_SRC, OUT / "base", BASE_TARGET) != 0:
        print(f"⚠ the base {base[:10]} failed some golden tests; the frames it did not draw "
              "read as new", flush=True)
    if not head_drawn and draw(ROOT, OUT / "head", TARGET) != 0:
        sys.exit("✗ the working tree's golden tests failed; the frames they did not draw "
                 "would read as removed, so there is no comparison to make")

    report = compare("-")
    if (OUT / "changed").read_text():
        if draw(ROOT, OUT / "again", TARGET) != 0:
            sys.exit("✗ the working tree's golden tests failed on their second draw")
        report = compare(str(OUT / "again"))
    return finish(base, report, args.strict)


def finish(base, report, strict):
    print(report, end="")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(f"## Golden frames against {base[:10]}\n\n{report}\n")
    return 1 if strict and (OUT / "changed").read_text() else 0


if __name__ == "__main__":
    sys.exit(main())
