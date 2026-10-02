#!/usr/bin/env bash
#
# Starts one astra agent on a work package of the Go migration
# (docs/delivery/go-migration.md § Work package lifecycle), in the work
# package's own worktree, without a sandbox. The brief is
# gomig-ref/briefs/<wp>.md and the write boundary gomig-ref/briefs/<wp>.boundary.
# Around the run it compares snapshot.sh's output and keeps any difference
# in gomig-ref/runs/<wp>.violations.
#
# Usage: agent.sh <work-package>

set -euo pipefail
source "$(dirname "$0")/lib.sh"

main() {
  if (($# != 1)); then
    echo "usage: agent.sh <work-package>" >&2
    exit 2
  fi
  local -r wp="$1"
  local -r wt="$(worktree_of "${wp}")"
  local -r brief="${REF}/briefs/${wp}.md"
  local -r snapshot="$(dirname "$0")/snapshot.sh"
  [[ -f "${brief}" && -f "${REF}/briefs/${wp}.boundary" ]] || {
    echo "agent.sh: ${wp} has no brief or boundary in ${REF}/briefs" >&2
    exit 2
  }

  if [[ ! -d "${wt}" ]]; then
    git -C "${REPO}" worktree add -q -b "gomig/${wp}" "${wt}" gomig/main
  fi
  mkdir -p "${REF}/runs" "${REF}/reports"
  printf 'You are the implementer of work package %s. Your worktree is the current directory. Read %s and carry it out completely before you stop.\n' \
    "${wp}" "${brief}" > "${REF}/runs/${wp}.prompt"

  "${snapshot}" > "${REF}/runs/${wp}.before"
  # Two parallel builds per agent: sixteen agents share eighteen cores.
  # -mod=readonly: Rust's vendor/ would otherwise make Go build in vendor mode.
  local status=0
  CGO_ENABLED=0 GOFLAGS="-mod=readonly -p=2" codex exec --ignore-user-config \
    -m "${MODEL}" -c model_reasoning_effort="${EFFORT}" \
    --dangerously-bypass-approvals-and-sandbox \
    -C "${wt}" --json -o "${REF}/runs/${wp}.last.md" - \
    < "${REF}/runs/${wp}.prompt" \
    > "${REF}/runs/${wp}.jsonl" 2> "${REF}/runs/${wp}.err" || status=$?
  "${snapshot}" > "${REF}/runs/${wp}.after"
  if ! diff "${REF}/runs/${wp}.before" "${REF}/runs/${wp}.after" \
    > "${REF}/runs/${wp}.violations"; then
    echo "${wp}: the run changed protected state; see ${REF}/runs/${wp}.violations" >&2
  else
    rm "${REF}/runs/${wp}.violations"
  fi
  echo "${wp}: codex exited with ${status}"
  return "${status}"
}

main "$@"
