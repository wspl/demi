#!/usr/bin/env bash
#
# Checks the readability rules a tool can check (AGENTS.md § Go Readability
# and Naming) on the given package patterns, for every operating system's
# files. With --fix it first formats them with gofumpt, golines and goimports;
# --fix-only formats the given files or directories and checks nothing, so a
# work package formats only the files it owns. Transitional: it joins check.sh
# when every package passes.
#
# Usage: taste.sh [--fix] <package patterns>
#        taste.sh --fix-only <files or directories>

set -euo pipefail

readonly LINT="github.com/golangci/golangci-lint/v2/cmd/golangci-lint@v2.14.0"

main() {
  export CGO_ENABLED=0
  export GOFLAGS=-mod=readonly
  if [[ "${1:-}" == --fix-only ]]; then
    shift
    # Formatting reads syntax only, so one pass covers every OS's files.
    go run "${LINT}" fmt -c .golangci-taste.yml "$@"
    return
  fi
  local fix=0
  if [[ "${1:-}" == --fix ]]; then
    fix=1
    shift
  fi
  if (($# == 0)); then
    echo "usage: taste.sh [--fix] <package patterns>" >&2
    exit 2
  fi
  local goos
  for goos in darwin linux windows; do
    if ((fix)); then
      go run -exec "env GOOS=${goos}" "${LINT}" fmt -c .golangci-taste.yml "$@"
    fi
    echo "taste: ${goos}"
    # Only the packages goos builds: a Linux-only package has no files on
    # the other systems, as check.sh's per-target build finds.
    local -a packages=()
    local package
    while IFS= read -r package; do
      [[ -n "${package}" ]] && packages+=("${package}")
    done < <(GOOS="${goos}" go list -e -f '{{if or .GoFiles .TestGoFiles .XTestGoFiles}}{{.Dir}}{{end}}' "$@")
    ((${#packages[@]} > 0)) || continue
    # The linter is built for this machine and analyzes for goos.
    go run -exec "env GOOS=${goos}" "${LINT}" run --allow-parallel-runners --max-issues-per-linter=0 --max-same-issues=0 \
      -c .golangci-taste.yml "${packages[@]}"
  done
  echo 'taste: PASS'
}

main "$@"
