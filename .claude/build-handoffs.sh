# Add preserved builds after registered worktrees. A local build wins; an
# unbuilt checkout falls back to its latest published handoff for that branch.
collect_handoffs() {
  local common catalog path branch commit created i found
  common="$(git rev-parse --path-format=absolute --git-common-dir)"
  [[ -d "$common/agent-lifecycle/builds" ]] || return 0
  catalog="$("$HERE_SCRIPT/session-lifecycle.sh" catalog)" || return 1
  while IFS=$'\t' read -r path branch commit created; do
    [[ -n "$path" ]] || continue
    found=0
    for i in "${!WT_BRANCH[@]}"; do
      if [[ "${WT_BRANCH[$i]}" == "$branch" ]]; then
        [[ -f "${WT_PATH[$i]}/target/release/lib${LIB}.dylib" ]] || WT_PATH[$i]="$path"
        found=1
        break
      fi
    done
    if [[ "$found" == 0 ]]; then
      WT_PATH+=("$path")
      WT_BRANCH+=("$branch")
    fi
  done <<< "$catalog"
}

# Published source identity comes from the checked manifest, not the checkout
# that may since have advanced or been removed.
build_commit() {
  if [[ -f "$1/handoff.json" ]]; then
    python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["commit"])' "$1/handoff.json"
  else
    git -C "$1" rev-parse HEAD
  fi
}

# The short form every message prints, or '?' when the source cannot be read:
# under `set -euo pipefail` a bare failing pipeline would end the loader
# silently, mid-load, or read as a fresh match for any stamped sha.
build_short() {
  build_commit "$1" 2>/dev/null | cut -c1-7 || echo '?'
}

build_commit_time() {
  if [[ -f "$1/handoff.json" ]]; then
    python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["commit_time"])' "$1/handoff.json"
  else
    git -C "$1" show -s --format=%ct HEAD
  fi
}
