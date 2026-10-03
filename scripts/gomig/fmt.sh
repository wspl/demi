#!/usr/bin/env bash
#
# Formats the given Go files or directories with the formatters .golangci.yml
# enables (gofumpt, golines, goimports), which check.sh then verifies.
# Formatting reads syntax only, so one pass covers every operating system's
# files.
#
# Usage: fmt.sh <files or directories>

set -euo pipefail

main() {
  if (($# == 0)); then
    echo "usage: fmt.sh <files or directories>" >&2
    exit 2
  fi
  CGO_ENABLED=0 GOFLAGS=-mod=readonly \
    go run github.com/golangci/golangci-lint/v2/cmd/golangci-lint@v2.14.0 fmt "$@"
}

main "$@"
