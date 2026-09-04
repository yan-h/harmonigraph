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
