#!/usr/bin/env bash
# Build the Rust runner for the managed Linux guest with static musl linkage.
set -euo pipefail
arch="${1:?Usage: runner/build.sh <aarch64|x86_64>}"
case "$arch" in
  aarch64|x86_64) ;;
  *)
    echo "unknown arch: $arch" >&2
    exit 2
    ;;
esac
here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$here/out/$arch"
mkdir -p "$out"
cargo zigbuild --manifest-path "$root/Cargo.toml" --locked --release \
  --target "$arch-unknown-linux-musl" -p demi-runner
install -m 0755 "${CARGO_TARGET_DIR:-$root/target}/$arch-unknown-linux-musl/release/demi-runner" "$out/demi-runner"
echo "$out/demi-runner"
