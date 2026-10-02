#!/usr/bin/env bash
# Does the security audit keep unlike triggers out of each other's cancellation
# group? A weekly or manually requested run always scans the lockfile, while a
# push to main is allowed to skip when the dependency set did not move. If they
# share a group, that unrelated push cancels the scan and replaces it with a
# green "Nothing to audit" run.
#
#   .claude/tests/audit-workflow.sh          # run it
#
# Written for bash 3.2 (macOS system bash), like the scripts beside it.
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORKFLOW="$ROOT/.github/workflows/audit.yml"

[ -f "$WORKFLOW" ] || {
  echo "✗ security-audit workflow is missing: $WORKFLOW" >&2
  exit 1
}

group=$(sed -n 's/^[[:space:]]*group:[[:space:]]*//p' "$WORKFLOW")
event_token='${{ github.event_name }}'

if [ -z "$group" ]; then
  echo "✗ security-audit workflow has no concurrency group" >&2
  exit 1
fi

case "$group" in
  *"$event_token"*)
    echo "✓ scheduled, manual, push, and PR audits have separate cancellation groups"
    ;;
  *)
    echo "✗ unlike security-audit triggers share one cancellation group:" >&2
    echo "    $group" >&2
    echo "    a main push can cancel a weekly/manual scan and then skip the audit" >&2
    exit 1
    ;;
esac

# Exercise the actual reachability step: failed dependency resolution is not
# evidence that a suppressed crate is absent.
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
python3 - "$WORKFLOW" > "$TMP/guard.sh" <<'PY'
from pathlib import Path
import sys
import textwrap
step = Path(sys.argv[1]).read_text().split("- name: Are the suppressed crates still unreachable?", 1)[1]
body = step.split("run: |\n", 1)[1].split("\n      - name:", 1)[0]
print(textwrap.dedent(body))
PY
if [ "$?" -ne 0 ] || [ ! -s "$TMP/guard.sh" ]; then
  echo "✗ could not extract the actual suppression guard" >&2
  exit 1
fi
cargo() {
  case "$AUDIT_TREE_CASE" in
    absent) printf 'harmonigraph-plugin v0.1.0\n' ;;
    present) printf 'harmonigraph-plugin v0.1.0\nquick-xml v0.40.0\n' ;;
    failed) return 101 ;;
  esac
}
export -f cargo
for scenario in absent present failed; do
  AUDIT_TREE_CASE="$scenario" bash "$TMP/guard.sh" > "$TMP/output" 2>&1
  result=$?
  if { [ "$scenario" = absent ] && [ "$result" -ne 0 ]; } || \
     { [ "$scenario" != absent ] && [ "$result" -eq 0 ]; }; then
    echo "✗ suppression guard mishandled $scenario (exit $result)" >&2
    cat "$TMP/output" >&2
    exit 1
  fi
done
echo "✓ suppressions require a successful dependency tree with no suppressed crate"
