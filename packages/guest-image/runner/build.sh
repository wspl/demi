#!/usr/bin/env bash
# Link the runner into the forked txiki.js runtime as a static Linux executable.
# Requires Bun, CMake, a native C/C++ compiler, and Zig (or ZIG=/path/to/zig).
set -euo pipefail
arch="${1:?Usage: runner/build.sh <aarch64|x86_64>}"
case "$arch" in
  aarch64|x86_64)
    ;;
  *)
    echo "unknown arch: $arch" >&2
    exit 2
    ;;
esac
here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$here/out/$arch"
mkdir -p "$out"
bun "$root/packages/runner/runtime/bundle.ts" \
  "$root/packages/runner/src/entry.ts" "$out/entry.mjs"
bun "$root/packages/runner/runtime/build.ts" \
  "$out/entry.mjs" "$out/demi-runner" "$arch-linux-musl"
bun "$root/packages/command-client/build.ts" "$arch-linux-musl"
install -m 0755 "$root/.cache/command-client/$arch-linux-musl/demi" "$out/demi"
echo "$out/demi-runner"
