# Shared settings of the Go migration's agent scripts
# (docs/delivery/go-migration.md). Sourced, not run.

# The main checkout, whichever worktree a script runs from.
REPO="$(cd "$(git rev-parse --git-common-dir)/.." && pwd)"
readonly REPO
readonly WORKTREES="${GOMIG_WORKTREES:-$(dirname "${REPO}")/demi-worktrees}"
readonly REF="${WORKTREES}/gomig-ref"
readonly MODEL="gpt-6-astra"
readonly EFFORT="low"

# Prints the worktree of a work package.
worktree_of() {
  echo "${WORKTREES}/gomig-$1"
}
