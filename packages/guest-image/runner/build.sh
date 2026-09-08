#!/usr/bin/env bash
# Link the runner into the forked txiki.js runtime as a static Linux executable.
# Requires Bun, CMake, a native C/C++ compiler, and Zig (or ZIG=/path/to/zig).
set -euo pipefail
arch="${1:?Usage: runner/build.sh <aarch64|x86_64>}"
case "$arch" in aarch64|x86_64) ;; *) echo "unknown arch: $arch" >&2; exit 2 ;; esac
here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$here/out/$arch"
mkdir -p "$out"
bun build "$root/packages/runner/src/entry.ts" --format=esm --target=browser --conditions=development --external 'tjs:*' --outfile "$out/entry.mjs"
bun "$root/packages/runner/runtime/build.ts" "$out/entry.mjs" "$out/demi-runner" "$arch-linux-musl"
echo "$out/demi-runner"
