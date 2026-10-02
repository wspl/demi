#!/usr/bin/env bash
#
# Starts one astra agent on a work package of the Go migration
# (docs/delivery/go-migration.md § Isolation rules). The agent runs in the
# work package's own worktree under Codex's workspace-write sandbox, which
# lets it write only the package directories given here and the Go caches.
#
# Usage: agent.sh <work-package> <brief> <dir> [<dir>...]
#   <work-package>  the work package id, such as r-shell
#   <brief>         the brief's path; the agent reads it first
#   <dir>           a directory of the write boundary, relative to the
#                   repository root; the first is the agent's working directory

set -euo pipefail

# The main checkout, whichever worktree this script runs from.
REPO="$(cd "$(git rev-parse --git-common-dir)/.." && pwd)"
readonly REPO
readonly WORKTREES="${GOMIG_WORKTREES:-$(dirname "${REPO}")/demi-worktrees}"
readonly REF="${WORKTREES}/gomig-ref"
readonly MODEL="gpt-6-astra"
readonly EFFORT="low"

main() {
  if (($# < 3)); then
    echo "usage: agent.sh <work-package> <brief> <dir> [<dir>...]" >&2
    exit 2
  fi
  local -r wp="$1"
  local -r brief="$2"
  shift 2
  local -r wt="${WORKTREES}/gomig-${wp}"

  if [[ ! -d "${wt}" ]]; then
    git -C "${REPO}" worktree add -q -b "gomig/${wp}" "${wt}" gomig/main
  fi

  local -a boundary=()
  local dir
  for dir in "$@"; do
    mkdir -p "${wt}/${dir}"
    boundary+=("${wt}/${dir}")
  done

  local -a add_dirs=(
    --add-dir "${HOME}/Library/Caches/go-build"
    --add-dir "${HOME}/go"
  )
  for dir in "${boundary[@]:1}"; do
    add_dirs+=(--add-dir "${dir}")
  done

  mkdir -p "${REF}/runs"
  # resume.sh restores the same write boundary from this file.
  printf '%s\n' "${boundary[@]}" > "${REF}/runs/${wp}.boundary"
  printf 'You are the implementer of work package %s. Read %s and carry it out completely before you stop.\n' \
    "${wp}" "${brief}" > "${REF}/runs/${wp}.prompt"

  # A parallel build per agent: sixteen agents share eighteen cores.
  CGO_ENABLED=0 GOFLAGS=-p=2 codex exec --ignore-user-config \
    -m "${MODEL}" -c model_reasoning_effort="${EFFORT}" \
    -s workspace-write -c sandbox_workspace_write.network_access=true \
    -C "${boundary[0]}" "${add_dirs[@]}" \
    --json -o "${REF}/runs/${wp}.last.md" - \
    < "${REF}/runs/${wp}.prompt" \
    > "${REF}/runs/${wp}.jsonl" 2> "${REF}/runs/${wp}.err"
}

main "$@"
