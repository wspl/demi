#!/usr/bin/env bash
#
# Checks the readability rules a tool can check (AGENTS.md § Go Readability
# and Naming) on the given package patterns, for every operating system's
# files. With --fix it first formats them with gofumpt, golines and goimports.
# Transitional: it joins check.sh when every package passes.
#
# Usage: taste.sh [--fix] <package patterns>

set -euo pipefail

readonly LINT="github.com/golangci/golangci-lint/v2/cmd/golangci-lint@v2.14.0"

main() {
  export CGO_ENABLED=0
  export GOFLAGS=-mod=readonly
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
    # The linter is built for this machine and analyzes for goos.
    go run -exec "env GOOS=${goos}" "${LINT}" run --allow-parallel-runners --max-issues-per-linter=0 --max-same-issues=0 \
      -c .golangci-taste.yml "$@"
  done
  echo 'taste: PASS'
}

main "$@"
