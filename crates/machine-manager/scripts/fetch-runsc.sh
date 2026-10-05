#!/usr/bin/env bash
# Fetches the pinned gVisor runtime distribution of one architecture,
# checks it against runtime/release.json and unpacks it into a new directory:
# upstream's release for amd64, and for arm64 the build with Demi's seccomp
# trap fix that the Runtime workflow published (builds-and-releases.md
# § gVisor runtime). A server release carries the result in its runtime/.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
release="$here/runtime/release.json"
[ "$#" -eq 2 ] || { echo 'usage: fetch-runsc.sh amd64|arm64 <new-directory>' >&2; exit 2; }
architecture=$1
output=$2
[ ! -e "$output" ] || { echo "$output exists already" >&2; exit 2; }
upstream=$(jq -er .upstream "$release")
case "$architecture" in
  amd64)
    url="https://storage.googleapis.com/gvisor/releases/release/$upstream/x86_64/gvisor.tar.bz2"
    digest=$(jq -er .amd64ArchiveSha512 "$release")
    ;;
  arm64)
    version=$(jq -er .arm64Version "$release")
    url="https://github.com/wspl/demi/releases/download/runsc-$version/gvisor-arm64.tar.bz2"
    digest=$(jq -er .arm64ArchiveSha512 "$release")
    ;;
  *) echo "unknown architecture: $architecture" >&2; exit 2 ;;
esac
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
curl -fsSL "$url" -o "$stage/gvisor.tar.bz2"
printf '%s  %s\n' "$digest" "$stage/gvisor.tar.bz2" | sha512sum -c - >&2
mkdir "$stage/runtime"
tar -xjf "$stage/gvisor.tar.bz2" -C "$stage/runtime"
chmod -R a+rX "$stage/runtime"
mkdir -p "$(dirname "$output")"
mv "$stage/runtime" "$output"
