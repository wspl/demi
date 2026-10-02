#!/usr/bin/env bash
#
# Prints the state no agent may change: the owner's checkout and every
# worktree outside the migration, every branch and tag outside gomig/, the
# remote-tracking branches and the stash. The migration's own worktrees and
# branches change while agents run, so a write there is caught by the
# boundary check of the package that owns it instead.
#
# Usage: snapshot.sh

set -euo pipefail
source "$(dirname "$0")/lib.sh"

main() {
  local path=""
  local line
  while IFS= read -r line; do
    case "${line}" in
      "worktree "*) path="${line#worktree }" ;;
      "HEAD "*)
        case "${path}" in
          "${WORKTREES}"/gomig-*) ;;
          *)
            echo "worktree ${path} ${line#HEAD }"
            git -C "${path}" status --porcelain | sed "s|^|  |"
            ;;
        esac
        ;;
    esac
  done < <(git -C "${REPO}" worktree list --porcelain)
  git -C "${REPO}" for-each-ref --format='%(refname) %(objectname)' \
    | grep -v -e '^refs/heads/gomig/' -e '^refs/remotes/origin/gomig/' || true
  git -C "${REPO}" stash list --format='stash %H %gs'
}

main "$@"
