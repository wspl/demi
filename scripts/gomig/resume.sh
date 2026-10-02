#!/usr/bin/env bash
#
# Sends review findings to a work package's agent in its own session, so it
# keeps its context. The agent's previous run must have exited: Codex refuses
# a second writer to a session.
#
# Usage: resume.sh <work-package> <findings-file>

set -euo pipefail

REPO="$(cd "$(git rev-parse --git-common-dir)/.." && pwd)"
readonly REPO
readonly WORKTREES="${GOMIG_WORKTREES:-$(dirname "${REPO}")/demi-worktrees}"
readonly REF="${WORKTREES}/gomig-ref"

main() {
  if (($# != 2)); then
    echo "usage: resume.sh <work-package> <findings-file>" >&2
    exit 2
  fi
  local -r wp="$1"
  local -r findings="$2"
  local session
  session="$(head -1 "${REF}/runs/${wp}.jsonl" \
    | python3 -c 'import json, sys; print(json.load(sys.stdin)["thread_id"])')"
  local -r round="$(date +%H%M%S)"
  # Read line by line: macOS ships bash 3.2, which has no mapfile.
  local -a boundary=()
  local line
  while IFS= read -r line; do
    boundary+=("${line}")
  done < "${REF}/runs/${wp}.boundary"
  local roots="\"${HOME}/Library/Caches/go-build\",\"${HOME}/go\""
  local dir
  for dir in "${boundary[@]:1}"; do
    roots+=",\"${dir}\""
  done

  cd "${boundary[0]}"
  CGO_ENABLED=0 GOFLAGS=-p=2 codex exec resume "${session}" \
    -m gpt-6-astra -c model_reasoning_effort=low \
    -c sandbox_mode='"workspace-write"' \
    -c sandbox_workspace_write.network_access=true \
    -c "sandbox_workspace_write.writable_roots=[${roots}]" \
    --json -o "${REF}/runs/${wp}.${round}.last.md" - \
    < "${findings}" \
    > "${REF}/runs/${wp}.${round}.jsonl" 2> "${REF}/runs/${wp}.${round}.err"
}

main "$@"
