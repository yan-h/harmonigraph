#!/usr/bin/env bash
# Do ci.sh's gate groups and ci.yml's job matrix still describe the same set of
# jobs, and does every gate belong to one?
#
# ci.sh splits its gates into groups so the workflow can run them as parallel
# jobs, which makes the group list THREE copies of one fact: `CI_GROUPS` in
# ci.sh, the `group` markers that claim the gates, and `matrix.group` in ci.yml.
# Nothing else in the tree notices when the copies disagree, and every way they
# can disagree is silent in the direction that matters — a group named in only
# some of the three is a set of gates NO job runs, and the PR still goes green
# because the jobs that do exist all passed.
#
# The remaining two cases are that same failure at its edges. A gate written
# above the first marker belongs to no group at all, and the workflow can stop
# passing the group entirely, which runs every gate in both legs for twice the
# macOS budget. Each is invisible: a bare `./ci.sh` locally runs everything, so
# whoever made the change watches it pass.
#
#   .claude/tests/ci-groups.sh          # run it
#
# Written for bash 3.2 (macOS system bash), like the scripts beside it.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CI="$ROOT/ci.sh"
WORKFLOW="$ROOT/.github/workflows/ci.yml"

for f in "$CI" "$WORKFLOW"; do
  [ -f "$f" ] || {
    echo "✗ missing: $f" >&2
    exit 1
  }
done

# `CI_GROUPS=(workspace isolated)` -> `workspace isolated`
declared=$(sed -n 's/^CI_GROUPS=(\(.*\))[[:space:]]*$/\1/p' "$CI")
if [ -z "$declared" ]; then
  echo "✗ ci.sh declares no CI_GROUPS array" >&2
  exit 1
fi

# `        group: [workspace, isolated]` -> `workspace isolated`
matrix=$(sed -n 's/^[[:space:]]*group:[[:space:]]*\[\(.*\)\][[:space:]]*$/\1/p' "$WORKFLOW" | tr -d ',')
if [ -z "$matrix" ]; then
  echo "✗ ci.yml has no matrix.group list" >&2
  exit 1
fi

# Compare as sets, so reordering either list is not a failure.
sorted_declared=$(echo "$declared" | tr ' ' '\n' | sed '/^$/d' | sort | tr '\n' ' ')
sorted_matrix=$(echo "$matrix" | tr ' ' '\n' | sed '/^$/d' | sort | tr '\n' ' ')

if [ "$sorted_declared" != "$sorted_matrix" ]; then
  echo "✗ ci.sh and ci.yml disagree about which groups exist:" >&2
  echo "    ci.sh  CI_GROUPS:    $sorted_declared" >&2
  echo "    ci.yml matrix.group: $sorted_matrix" >&2
  echo "    a group missing from the matrix runs nowhere, and CI still reports green" >&2
  exit 1
fi

# The markers are the THIRD copy of the same fact, and they drift both ways.
# A declared group that marks no gates is a job that spins up a macOS runner,
# restores the cache and runs nothing — invisible, because an empty job passes.
# A marker naming a group nobody declared is worse, and is the likely shape of
# a botched third group: every gate under it runs in NO job, while `./ci.sh`
# locally still runs them under `all`, so whoever added the section watches
# them pass. Comparing ci.sh's list against ci.yml's cannot see either case,
# because neither list mentions the markers.
sorted_markers=$(sed -n 's/^group \(.*\)$/\1/p' "$CI" | sort -u | tr '\n' ' ')

if [ "$sorted_markers" != "$sorted_declared" ]; then
  echo "✗ ci.sh's 'group' markers and its CI_GROUPS list disagree:" >&2
  echo "    markers:   ${sorted_markers% }" >&2
  echo "    CI_GROUPS: ${sorted_declared% }" >&2
  echo "    a marker naming an undeclared group hides every gate under it from CI" >&2
  exit 1
fi

# And the workflow has to actually PASS the group. A regression to a bare
# `./ci.sh` is easy — it is what the line said before the split, so a merge
# resolution can put it back — and it is silent in the expensive direction:
# both legs then run every gate, taking twice the macOS budget this split
# exists to protect, with every check still green.
if ! grep -qF './ci.sh ${{ matrix.group }}' "$WORKFLOW"; then
  echo "✗ ci.yml does not pass the matrix group to ci.sh" >&2
  echo "    without it both legs run every gate, greenly and at twice the cost" >&2
  exit 1
fi

# And no gate may sit above the first marker. `run` is the only way a gate is
# invoked, plus the one `if in_group` block, so the first of either has to come
# after the first `group` line.
first_marker=$(grep -n '^group ' "$CI" | head -1 | cut -d: -f1)
first_gate=$(grep -n '^run \|^if in_group; then$' "$CI" | head -1 | cut -d: -f1)

if [ -z "$first_marker" ]; then
  echo "✗ ci.sh has no 'group' markers at all" >&2
  exit 1
fi

if [ -n "$first_gate" ] && [ "$first_gate" -lt "$first_marker" ]; then
  echo "✗ ci.sh runs a gate at line $first_gate, before the first group marker at line $first_marker" >&2
  echo "    that gate belongs to no group, so it runs locally and in no CI job" >&2
  exit 1
fi

echo "✓ ci.sh groups and ci.yml matrix agree (${sorted_declared% }), and every gate is in one"
