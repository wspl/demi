#!/usr/bin/env bash
# Says whether a build run's files are all kept, and how many debug-file
# artifacts it has.
#
# Usage: build-files.sh RUN_ID
#
# The files of a release are the run's artifacts: each architecture's server
# and image, and every program's debug files. They are kept one day,
# each from its own upload, so some can expire while others are still there,
# and a download by pattern skips an expired artifact without an error. The
# build's files are therefore all kept only when each of those artifacts is
# kept for two more hours, which is more than a release takes to download them.
#
# Prints the number of the run's `symbols-*` artifacts and exits 0 when the
# files are all kept. Otherwise says what is missing on stderr and exits 1.

set -euo pipefail

readonly margin_seconds=7200
readonly required=(server-amd64 server-arm64 image-amd64 image-arm64)

main() {
  local run=$1 artifacts name symbols
  artifacts=$(gh api --paginate "repos/$GITHUB_REPOSITORY/actions/runs/$run/artifacts" \
    --jq ".artifacts[]
      | select(.name | test(\"^(server|image|symbols)-\"))
      | \"\\(.name) \\((.expired | not) and ((.expires_at // \"1970-01-01T00:00:00Z\" | fromdateiso8601) > now + $margin_seconds))\"")
  for name in "${required[@]}"; do
    if ! grep -qx "$name true" <<< "$artifacts"; then
      echo "run $run: $name is missing, expired or about to expire" >&2
      return 1
    fi
  done
  if grep -q ' false$' <<< "$artifacts"; then
    echo "run $run: $(grep ' false$' <<< "$artifacts" | cut -d' ' -f1 | tr '\n' ' ')expired or about to expire" >&2
    return 1
  fi
  symbols=$({ grep '^symbols-' <<< "$artifacts" || true; } | cut -d' ' -f1 | sort -u | awk 'END { print NR }')
  if [[ $symbols -eq 0 ]]; then
    echo "run $run: no debug files" >&2
    return 1
  fi
  echo "$symbols"
}

main "$@"
