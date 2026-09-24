#!/usr/bin/env bash
# Installs the pinned Cloud runtime distribution, sidecars included, under
# /opt/gvisor and prints the path of its runsc; with --path, prints that path
# without installing anything. `runtime/release.json` pins the release.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
release="$here/runtime/release.json"
# Only the runsc path goes to standard output; progress goes to standard error.
exec 3>&1 1>&2
upstream=$(jq -er .upstream "$release")
arm_version=$(jq -er .arm64Version "$release")
digest=$(jq -er .amd64ArchiveSha512 "$release")
arch=$(uname -m)
case "$arch" in
  aarch64) expected="$arm_version" ;;
  x86_64) expected="release-$upstream" ;;
  *) echo "Unsupported Cloud architecture: $arch" >&2; exit 1 ;;
esac
destination="/opt/gvisor/${expected#release-}"
if [ "${1:-}" = --path ]; then
  echo "$destination/runsc" >&3
  exit 0
fi
[ "$#" -eq 0 ] || { echo 'usage: install-runsc.sh [--path]' >&2; exit 2; }
# The manager accepts only the exact pinned version line.
reports_pinned() {
  [ "$("$1" --version | head -n 1)" = "runsc version $expected" ]
}
if [ -x "$destination/runsc" ]; then
  reports_pinned "$destination/runsc" || { echo "$destination/runsc is not runsc $expected" >&2; exit 1; }
  echo "$destination/runsc" >&3
  exit 0
fi
mkdir -p /opt/gvisor
stage=$(mktemp -d /opt/gvisor/.install-XXXXXX)
trap 'rm -rf "$stage"' EXIT
if [ "$arch" = aarch64 ]; then
  bash "$here/scripts/build-runsc-arm64.sh" "$stage/runtime"
else
  curl -fsSL "https://storage.googleapis.com/gvisor/releases/release/$upstream/$arch/gvisor.tar.bz2" -o "$stage/gvisor.tar.bz2"
  printf '%s  %s\n' "$digest" "$stage/gvisor.tar.bz2" | sha512sum -c -
  mkdir "$stage/runtime"
  tar -xjf "$stage/gvisor.tar.bz2" -C "$stage/runtime"
fi
chmod -R a+rX "$stage/runtime"
reports_pinned "$stage/runtime/runsc" || { echo "the built runsc is not runsc $expected" >&2; exit 1; }
mv "$stage/runtime" "$destination"
echo "$destination/runsc" >&3
