#!/usr/bin/env bash
# Prints a commit's version: the series and the number of commits in its
# history, `0.2.4213` for a commit with 4,213 commits behind it. The same commit
# always has the same version, and a later commit on a branch a higher one.
#
# Usage: version.sh [COMMIT]
#
# COMMIT defaults to HEAD. The checkout must hold the whole history of commits,
# not their files (`fetch-depth: 0`, `filter: tree:0` in a workflow).
#
# The series is the version's first two numbers and moves only here: a Beta
# that holds a new feature or a change on screen moves it to the next minor
# (docs/delivery/upgrades.md).

set -euo pipefail

readonly series=0.2

echo "$series.$(git rev-list --count "${1:-HEAD}")"
