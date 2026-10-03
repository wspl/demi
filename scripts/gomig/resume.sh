#!/usr/bin/env bash
#
# Sends review findings to a work package's agent in its own session, so it
# keeps its context. The agent's previous run must have exited: Codex refuses
# a second writer to a session. Snapshots are compared as in agent.sh.
#
# Usage: resume.sh <work-package> <findings-file>

set -euo pipefail
source "$(dirname "$0")/lib.sh"

main() {
  if (($# != 2)); then
    echo "usage: resume.sh <work-package> <findings-file>" >&2
    exit 2
  fi
  local -r wp="$1"
  local -r findings="$2"
  local -r wt="$(worktree_of "${wp}")"
  local model effort
  read -r model effort <<< "$(model_of "${wp}")"
  local -r snapshot="$(dirname "$0")/snapshot.sh"
  local session
  session="$(head -1 "${REF}/runs/${wp}.jsonl" \
    | python3 -c 'import json, sys; print(json.load(sys.stdin)["thread_id"])')"
  local -r round="${wp}.$(date +%H%M%S)"

  "${snapshot}" > "${REF}/runs/${round}.before"
  local status=0
  cd "${wt}"
  CGO_ENABLED=0 GOFLAGS="-mod=readonly -p=2" codex exec resume "${session}" \
    -m "${model}" -c model_reasoning_effort="${effort}" \
    --dangerously-bypass-approvals-and-sandbox \
    --json -o "${REF}/runs/${round}.last.md" - \
    < "${findings}" \
    > "${REF}/runs/${round}.jsonl" 2> "${REF}/runs/${round}.err" || status=$?
  "${snapshot}" > "${REF}/runs/${round}.after"
  if ! diff "${REF}/runs/${round}.before" "${REF}/runs/${round}.after" \
    > "${REF}/runs/${round}.violations"; then
    echo "${wp}: the run changed protected state; see ${REF}/runs/${round}.violations" >&2
  else
    rm "${REF}/runs/${round}.violations"
  fi
  echo "${wp}: codex exited with ${status}"
  return "${status}"
}

main "$@"
