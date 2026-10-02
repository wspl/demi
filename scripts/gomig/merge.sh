#!/usr/bin/env bash
#
# Merges a branch into a worktree. When the only conflicts are in go.mod and
# go.sum, which two work packages that each add a module always produce, it
# keeps both sides and lets `go mod tidy` settle them; any other conflict stops
# the merge with the conflicted paths listed. The tech lead uses it to merge a
# work package into gomig-lead and to bring gomig/main into a work package's
# worktree before resuming its agent. Transitional: it goes with the migration.
#
# Usage: merge.sh <worktree> <ref> [message]

set -euo pipefail

main() {
  if (($# < 2 || $# > 3)); then
    echo "usage: merge.sh <worktree> <ref> [message]" >&2
    exit 2
  fi
  local -r worktree="$1"
  local -r ref="$2"
  local -a args=(merge -q --no-ff)
  if (($# == 3)); then
    args+=(-m "$3")
  else
    args+=(--no-edit)
  fi
  if git -C "${worktree}" "${args[@]}" "${ref}"; then
    return 0
  fi
  local conflicts
  conflicts="$(git -C "${worktree}" diff --name-only --diff-filter=U)"
  if [[ -z "${conflicts}" ]] || grep -qvxE 'go\.(mod|sum)' <<< "${conflicts}"; then
    echo "merge.sh: conflicts beyond go.mod and go.sum in ${worktree}:" >&2
    echo "${conflicts}" >&2
    exit 1
  fi
  local file
  while read -r file; do
    python3 - "${worktree}/${file}" <<'EOF'
import re, sys
path = sys.argv[1]
text = open(path).read()
text = re.sub(r'<<<<<<< [^\n]*\n(.*?)=======\n(.*?)>>>>>>> [^\n]*\n',
              lambda m: m.group(1) + m.group(2), text, flags=re.S)
open(path, 'w').write(text)
EOF
  done <<< "${conflicts}"
  (cd "${worktree}" && CGO_ENABLED=0 GOFLAGS=-mod=mod go mod tidy)
  git -C "${worktree}" add go.mod go.sum
  git -C "${worktree}" commit -q --no-edit
}

main "$@"
