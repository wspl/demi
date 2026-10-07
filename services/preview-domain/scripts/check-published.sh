#!/usr/bin/env bash
# Refuses a push that changes or removes a file of a published version of the
# preview domain's static files (docs/delivery/builds-and-releases.md
# § Preview domain deployment): Demi pages built against a version may still
# be open, so its files never change or disappear. A new file in a published
# version breaks nothing and passes.
#
# Usage: check-published.sh <before> [<after>]
#   <before>  the commit the push started from
#   <after>   the commit it ends at; HEAD when omitted
set -euo pipefail

readonly VERSIONS="services/preview-domain/static/__demi"

main() {
  local before="$1"
  local after="${2:-HEAD}"
  local refused=0
  local version changes
  for version in $(git ls-tree --name-only "${before}" "${VERSIONS}/"); do
    # Every change but an addition; a rename counts as a removal.
    changes="$(git diff --no-renames --name-status --diff-filter=a "${before}" "${after}" -- "${version}")"
    if [[ -n "${changes}" ]]; then
      echo "::error::${version} was published, and its files must not change or disappear; publish the change as a new version" >&2
      echo "${changes}" >&2
      refused=1
    fi
  done
  return "${refused}"
}

main "$@"
