#!/usr/bin/env bash
# The packed runner for the guest: the runner bundle over a static musl
# tinyjs built with the guest's trust roots, packed by the host's tinyjsc.
# Usage: runner/build.sh <aarch64|x86_64>
set -euo pipefail
arch="${1:?arch}"
here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$here/out/$arch"
mkdir -p "$out"
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
target="$arch-unknown-linux-musl"
(cd "$root/packages/tinyjs" && if [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = "$arch" ]; then cargo build --release --target "$target" --bin tinyjs --features guest-roots; else cargo zigbuild --release --target "$target" --bin tinyjs --features guest-roots; fi)
(cd "$root/packages/tinyjs" && cargo build --release --bin tinyjsc)
bun build "$root/packages/runner/src/entry.ts" --format=esm --target=browser --conditions=development --external 'tinyjs:*' --outfile "$out/entry.mjs"
"$root/packages/tinyjs/target/release/tinyjsc" "$out/entry.mjs" --bin "$root/packages/tinyjs/target/$target/release/tinyjs" --out "$out/demi-runner"
rm "$out/entry.mjs"
echo "$out/demi-runner"
