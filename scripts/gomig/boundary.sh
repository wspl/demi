#!/usr/bin/env bash
#
# Checks that a work package changed nothing outside its write boundary
# (docs/delivery/go-migration.md § Isolation rules): every path changed on
# its branch since gomig/main, committed or not, must lie under one of the
# entries of gomig-ref/briefs/<wp>.boundary and under none of its exclusions,
# the entries that start with "!". A change to go.mod or go.sum is listed
# for the review, which checks it against the modules the brief allows.
#
# Usage: boundary.sh <work-package>

set -euo pipefail
source "$(dirname "$0")/lib.sh"

# Succeeds when the path is the entry or lies below it.
under() {
  [[ "$1" == "$2" || "$1" == "${2%/}/"* ]]
}

main() {
  if (($# != 1)); then
    echo "usage: boundary.sh <work-package>" >&2
    exit 2
  fi
  local -r wp="$1"
  local -r wt="$(worktree_of "${wp}")"
  local -a boundary=()
  local -a excluded=()
  local line
  while IFS= read -r line; do
    case "${line}" in
      "") ;;
      "!"*) excluded+=("${line#!}") ;;
      *) boundary+=("${line}") ;;
    esac
  done < "${REF}/briefs/${wp}.boundary"

  local outside=0
  local path entry inside
  while IFS= read -r path; do
    [[ -z "${path}" ]] && continue
    if [[ "${path}" == go.mod || "${path}" == go.sum ]]; then
      echo "review: ${path}"
      continue
    fi
    inside=0
    for entry in "${boundary[@]}"; do
      if under "${path}" "${entry}"; then
        inside=1
      fi
    done
    # bash 3.2 treats an empty array as unset under set -u.
    for entry in ${excluded[@]+"${excluded[@]}"}; do
      if under "${path}" "${entry}"; then
        inside=0
      fi
    done
    if ((inside == 0)); then
      echo "outside: ${path}"
      outside=1
    fi
  done < <(
    {
      git -C "${wt}" diff --name-only gomig/main...HEAD
      git -C "${wt}" status --porcelain --untracked-files=all | cut -c4-
    } | sort -u
  )
  if ((outside == 1)); then
    echo "boundary check failed for ${wp}" >&2
    exit 1
  fi
  echo "boundary check passed for ${wp}"
}

main "$@"
