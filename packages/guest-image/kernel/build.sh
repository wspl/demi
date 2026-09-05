#!/usr/bin/env bash
# The guest kernel: Linux 6.1 LTS on Firecracker's own microvm config for
# the series, with extra.config merged in. Usage: kernel/build.sh <aarch64|x86_64>
set -euo pipefail
arch="${1:?arch}"
here="$(cd "$(dirname "$0")/.." && pwd)"
out="$here/out/$arch"
work="${KERNEL_WORK:-$here/out/kernel-src}"
version="${KERNEL_VERSION:-6.1.155}"
fc_tag="${FIRECRACKER_TAG:-v1.16.1}"
mkdir -p "$out" "$work"
cd "$work"
if [ ! -d "linux-$version" ]; then
  curl -fsSL "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-$version.tar.xz" | tar -xJ
fi
cd "linux-$version"
curl -fsSL "https://raw.githubusercontent.com/firecracker-microvm/firecracker/$fc_tag/resources/guest_configs/microvm-kernel-ci-$arch-6.1.config" -o .config
./scripts/kconfig/merge_config.sh -m .config "$here/kernel/extra.config"
make olddefconfig
make -j"$(nproc)" vmlinux
if [ "$arch" = aarch64 ]; then
  make -j"$(nproc)" Image
  cp arch/arm64/boot/Image "$out/vmlinux"
else
  cp vmlinux "$out/vmlinux"
fi
echo "$out/vmlinux"
