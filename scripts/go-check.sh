#!/usr/bin/env bash
# Runs the Go checkpoint checks (docs/internal is not required).
#
# Usage: scripts/go-check.sh [base-ref]
# Tests run with the race detector for the packages changed since base-ref
# (default go/main), and for every package when base-ref is "all".
set -euo pipefail

cd "$(dirname "$0")/.."
base="${1:-go/main}"

# The generated contract bindings exist once the generator is in place (F4b).
if grep -q '"go:contracts"' package.json; then
  bun run go:contracts >/dev/null
fi

# cmd/ and third_party/ appear with their first work package; gofmt only
# complains that a missing directory does not exist.
format_dirs=()
for dir in cmd internal third_party; do
  if [[ -d "${dir}" ]]; then
    format_dirs+=("${dir}")
  fi
done
unformatted="$(gofmt -l "${format_dirs[@]}")"
if [[ -n "${unformatted}" ]]; then
  echo "gofmt: unformatted files:" >&2
  echo "${unformatted}" >&2
  exit 1
fi

go vet ./...
golangci-lint run ./...
go build ./...

if [[ "${base}" == "all" ]]; then
  packages=(./...)
else
  mapfile -t dirs < <(git diff --name-only "${base}" -- '*.go' 'go.mod' \
    | xargs -r -n1 dirname | sort -u)
  packages=()
  for dir in "${dirs[@]}"; do
    if [[ "${dir}" == "." ]]; then
      packages=(./...)
      break
    fi
    if [[ -d "${dir}" ]]; then
      packages+=("./${dir}/...")
    fi
  done
fi

if (( ${#packages[@]} > 0 )); then
  go test -race -count=1 "${packages[@]}"
fi
