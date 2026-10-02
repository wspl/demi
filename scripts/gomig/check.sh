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

step='vet'
go vet "$@"
step='golangci-lint'
go run github.com/golangci/golangci-lint/v2/cmd/golangci-lint@v2.14.0 run "$@"
step='sumtype'
go run github.com/alecthomas/go-check-sumtype/cmd/go-check-sumtype@v0.5.0 -default-signifies-exhaustive=false "$@"
step='architecture'
if [[ "${GOMIG_ARCHCHECK_FIXTURE_ONLY:-0}" == 1 ]]; then
  # Phase 0 only: the brief permits fixture tests until d-packages lands.
  if grep -q '^### Go packages$' docs/architecture/crates-and-packages.md; then
    echo 'fixture-only architecture check is forbidden when the Go graph exists' >&2
    exit 1
  fi
  echo 'gomig: Go graph absent; running archcheck fixture tests (Phase 0)'
  go test ./tools/archcheck
else
  go run ./tools/archcheck
fi
step='cgo'
go run ./tools/cgocheck
step='race tests'
if [[ "$(go env GOHOSTOS)" == linux ]]; then
  CGO_ENABLED=1 go test -race -tags netgo,osusergo "$@"
else
  go test -race "$@"
fi
echo 'gomig: PASS'
