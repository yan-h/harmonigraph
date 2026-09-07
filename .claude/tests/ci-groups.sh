#!/usr/bin/env bash
# Do ci.sh's gate groups and ci.yml's job matrix still describe the same set of
# jobs, and does every gate belong to one?
#
# ci.sh splits its gates into groups so the workflow can run them as parallel
# jobs, which makes the group list two copies of one fact: `CI_GROUPS` in ci.sh
# and `matrix.group` in ci.yml. Nothing else in the tree notices when the copies
# disagree, and every way they can disagree is silent in the direction that
# matters — a group named only in ci.sh is a set of gates NO job runs, and the
# PR still goes green because the jobs that do exist all passed.
#
# The third case is the same failure one line higher up: a gate written above
# the first `group` marker belongs to no group at all, so it runs for a bare
# `./ci.sh` locally and in none of the CI jobs. Whoever adds it sees it pass.
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

# Every declared group has to actually own gates. A group in both lists but on
# no marker is a job that spins up a macOS runner, restores the cache and runs
# nothing — invisible, because an empty job passes.
for g in $declared; do
  if ! grep -q "^group $g\$" "$CI"; then
    echo "✗ group '$g' is declared and matrixed but marks no gates in ci.sh" >&2
    exit 1
  fi
done

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
