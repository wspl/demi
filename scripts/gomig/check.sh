#!/usr/bin/env bash
# Runs the migration guard rails on the supplied Go package patterns.
set -euo pipefail

export CGO_ENABLED=0
export GOFLAGS=-mod=readonly

step='initialization'
trap 'echo "gomig check failed: ${step}" >&2' ERR

if (($# == 0)); then
  echo 'usage: scripts/gomig/check.sh <package patterns>' >&2
  exit 2
fi

for target in darwin/amd64 darwin/arm64 linux/amd64 linux/arm64 windows/amd64 windows/arm64; do
  step="build ${target}"
  echo "gomig: ${step}"
  # Expand patterns per target: a Linux-only package is absent on other targets.
  selected=$(GOOS="${target%/*}" GOARCH="${target#*/}" go list "$@")
  packages=()
  while IFS= read -r package; do
    if [[ -n "${package}" ]]; then
      packages+=("${package}")
    fi
  done <<< "${selected}"
  if ((${#packages[@]} > 0)); then
    GOOS="${target%/*}" GOARCH="${target#*/}" go build -o /dev/null "${packages[@]}"
  fi
done

step='generated'
echo "gomig: ${step}"
go run ./tools/contractgen -check "$@"

step='file names'
# The module layout forbids catch-all files (crates-and-packages.md § Module
# layout): a file is named for its one responsibility.
catch_all=$(find cmd internal tools scripts -name '*.go' \( -name 'util*.go' -o -name 'helper*.go' -o -name 'misc*.go' -o -name 'common*.go' \) -not -path '*/testdata/*')
if [[ -n "${catch_all}" ]]; then
  echo "catch-all files: ${catch_all}" >&2
  false
fi
step='vet'
go vet "$@"
step='golangci-lint'
go run github.com/golangci/golangci-lint/v2/cmd/golangci-lint@v2.14.0 run "$@"
step='sumtype'
go run github.com/alecthomas/go-check-sumtype/cmd/go-check-sumtype@v0.5.0 -default-signifies-exhaustive=false "$@"
step='architecture'
go run ./tools/archcheck
step='cgo'
go run ./tools/cgocheck
step='race tests'
if [[ "$(go env GOHOSTOS)" == linux ]]; then
  CGO_ENABLED=1 go test -race -tags netgo,osusergo "$@"
else
  go test -race "$@"
fi
step='shell fork'
# The patched interpreter is its own module, so ./... does not reach its
# tests; a whole-tree check runs them too.
for pattern in "$@"; do
  if [[ "${pattern}" == ./... ]]; then
    (cd third_party/mvdan-sh && go test -race ./...)
    break
  fi
done
echo 'gomig: PASS'
