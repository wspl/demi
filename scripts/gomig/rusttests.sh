#!/usr/bin/env bash
#
# Lists every Rust test of a crate as <file>::<function>, the checklist a work
# package's report maps to Go tests (docs/delivery/go-migration.md § Fidelity).
# Transitional: it goes with the Rust code.
#
# Usage: rusttests.sh <crate directory name>, such as shared-gates

set -euo pipefail

main() {
  if (($# != 1)); then
    echo "usage: rusttests.sh <crate>" >&2
    exit 2
  fi
  local -r crate="crates/$1"
  [[ -d "${crate}" ]] || { echo "rusttests.sh: no ${crate}" >&2; exit 2; }
  find "${crate}" -name '*.rs' -print0 | sort -z | xargs -0 awk '
    FNR == 1 { pending = 0 }
    /^[[:space:]]*#\[(tokio::)?test(\(|\]|[[:space:]])/ { pending = 1; next }
    pending && /^[[:space:]]*(pub[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+[A-Za-z0-9_]+/ {
      match($0, /fn[[:space:]]+[A-Za-z0-9_]+/)
      name = substr($0, RSTART, RLENGTH)
      sub(/fn[[:space:]]+/, "", name)
      print FILENAME "::" name
      pending = 0
      next
    }
    pending && /^[[:space:]]*#\[/ { next }
  '
}

main "$@"
