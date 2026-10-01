#!/usr/bin/env bash
# Build the pinned ARM runtime with Linux-compatible SECCOMP_RET_TRAP registers.
# Uses the execution host's native compiler and Bazel, never a build container.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
release="$here/runtime/release.json"
output="${1:?Usage: build-runsc-arm64.sh <new-output-directory>}"
[ "$(uname -m)" = aarch64 ] && [ "$(uname -s)" = Linux ] || { echo 'requires native Linux arm64' >&2; exit 2; }
[ "$(id -u)" = 0 ] || { echo 'run as root for the isolated runtime probe' >&2; exit 2; }
[ ! -e "$output" ] || { echo 'output already exists' >&2; exit 2; }
for tool in git curl jq python3 gcc aarch64-linux-gnu-gcc x86_64-linux-gnu-gcc clang pkg-config patch unshare; do
  command -v "$tool" >/dev/null || { echo "missing build tool: $tool" >&2; exit 2; }
done
upstream=$(jq -er .upstream "$release")
commit=$(jq -er .commit "$release")
version=$(jq -er .arm64Version "$release")
patch_hash=$(jq -er .arm64PatchSha256 "$release")
bazel_version=$(jq -er .bazel "$release")
bazel_hash=$(jq -er .bazelArm64Sha256 "$release")
printf '%s  %s\n' "$patch_hash" "$here/runtime/arm64-seccomp-trap.patch" | sha256sum -c -
work=$(mktemp -d)
probe_id="demi-abi-$$"
cleanup() {
  local status=$?
  if [ -d "$work/state" ]; then
    "$work/runtime/runsc" --root="$work/state" delete --force "$probe_id" || status=1
  fi
  rm -rf "$work"
  exit "$status"
}
trap cleanup EXIT
curl -fsSL "https://releases.bazel.build/$bazel_version/release/bazel-$bazel_version-linux-arm64" -o "$work/bazel"
printf '%s  %s\n' "$bazel_hash" "$work/bazel" | sha256sum -c -
chmod 0755 "$work/bazel"
git clone --depth 1 --branch "release-$upstream" https://github.com/google/gvisor.git "$work/source"
[ "$(git -C "$work/source" rev-parse HEAD)" = "$commit" ] || { echo 'gVisor source revision mismatch' >&2; exit 1; }
git -C "$work/source" apply "$here/runtime/arm64-seccomp-trap.patch"
printf '#!/bin/sh\nprintf "STABLE_VERSION %s\\n"\n' "$version" > "$work/version.sh"
chmod 0755 "$work/version.sh"
(cd "$work/source" && "$work/bazel" --host_jvm_args=-Xmx1024m build -c opt --config=aarch64 \
  --jobs=4 --local_resources=memory=4096 --workspace_status_command="$work/version.sh" //debian:gvisor-release-tar-bz2)
archive="$work/source/bazel-bin/debian/gvisor.tar.bz2"
[ -f "$archive" ] || { echo 'runtime distribution was not produced' >&2; exit 1; }
mkdir "$work/runtime"
tar -xjf "$archive" -C "$work/runtime"
"$work/runtime/runsc" --version
# A static probe distinguishes the fixed syscall ABI from the original ARM bug.
gcc -O2 -static "$here/runtime/seccomp-trap.c" -o "$work/probe"
"$work/probe"
mkdir -p "$work/bundle/rootfs"
cp "$work/probe" "$work/bundle/rootfs/probe"
cat > "$work/bundle/config.json" <<'JSON'
{"ociVersion":"1.1.0","root":{"path":"rootfs","readonly":true},"process":{"terminal":false,"user":{"uid":1000,"gid":1000},"args":["/probe"],"env":[],"cwd":"/","noNewPrivileges":true},"linux":{"namespaces":[{"type":"pid"},{"type":"mount"},{"type":"ipc"},{"type":"uts"},{"type":"network"}]}}
JSON
# runsc's network=none probe pins a network namespace; keep that mount private.
unshare --mount --propagation private timeout 60 "$work/runtime/runsc" --root="$work/state" --platform=systrap --network=none run --bundle="$work/bundle" "$probe_id"
"$work/runtime/runsc" --root="$work/state" delete --force "$probe_id"
rm -rf "$work/state"
sha512sum "$archive" > "$work/runtime/build-archive.sha512"
cp "$release" "$work/runtime/demi-build.json"
mkdir -p "$(dirname "$output")"
mv "$work/runtime" "$output"
