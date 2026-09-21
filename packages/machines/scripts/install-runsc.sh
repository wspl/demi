#!/usr/bin/env bash
# Install the exact Cloud runtime distribution, including its sidecars.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
bun="${BUN:-bun}"
IFS=$'\t' read -r version arm_version digest < <("$bun" -e '
const r = await Bun.file(process.argv[1]).json();
console.log([r.upstream,r.arm64Version,r.amd64ArchiveSha512].join("\t"));
' "$here/runtime/release.json")
arch=$(uname -m)
case "$arch" in
  aarch64) expected="$arm_version" ;;
  x86_64) expected="release-$version" ;;
  *) echo "Unsupported Cloud architecture: $arch" >&2; exit 1 ;;
esac
destination="/opt/gvisor/${expected#release-}"
if [ -x "$destination/runsc" ]; then
  "$destination/runsc" --version | grep -F "runsc version $expected"
  exit 0
fi
mkdir -p /opt/gvisor
stage=$(mktemp -d /opt/gvisor/.install-XXXXXX)
trap 'rm -rf "$stage"' EXIT
if [ "$arch" = aarch64 ]; then
  BUN="$bun" bash "$here/scripts/build-runsc-arm64.sh" "$stage/runtime"
else
  curl -fsSL "https://storage.googleapis.com/gvisor/releases/release/$version/$arch/gvisor.tar.bz2" -o "$stage/gvisor.tar.bz2"
  printf '%s  %s\n' "$digest" "$stage/gvisor.tar.bz2" | sha512sum -c -
  mkdir "$stage/runtime"
  tar -xjf "$stage/gvisor.tar.bz2" -C "$stage/runtime"
fi
chmod -R a+rX "$stage/runtime"
"$stage/runtime/runsc" --version | grep -F "runsc version $expected"
mv "$stage/runtime" "$destination"
