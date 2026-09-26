#!/usr/bin/env bash
# Builds a Cloud image release (images.md § Build pipeline) as root on a Linux
# builder, for the builder's architecture. This script is the first stage:
# Ubuntu 26.04 by debootstrap, the toolchain from packages.txt and tini, the
# guest user `demi` (uid 1000) with passwordless sudo, and the file overlay.
# The second stage is `xtask cloud-image package`, built for this builder on
# the developer's machine: it embeds the runner, the command packages, Chrome
# for Testing and uv, and publishes the verified root archive and manifest.
#
# Usage: sudo bash rootfs/build.sh --xtask PATH --runners DIR --package DIR...
#          --output DIR [--work DIR] [--mirror URL]
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
xtask=""
runners=""
packages=()
output=""
work=/var/tmp/demi-cloud-root
mirror=""
usage() {
  echo 'usage: build.sh --xtask PATH --runners DIR --package DIR [--package DIR]...' >&2
  echo '         --output DIR [--work DIR] [--mirror URL]' >&2
  exit 2
}
while [ "$#" -gt 0 ]; do
  [ "$#" -ge 2 ] || usage
  case "$1" in
    --xtask) xtask=$2 ;;
    --runners) runners=$2 ;;
    --package) packages+=(--package "$2") ;;
    --output) output=$2 ;;
    --work) work=$2 ;;
    --mirror) mirror=$2 ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
  shift 2
done
[ -n "$xtask" ] && [ -n "$runners" ] && [ "${#packages[@]}" -gt 0 ] && [ -n "$output" ] || usage
[ -x "$xtask" ] || { echo "no xtask executable at $xtask" >&2; exit 2; }
# A release is immutable: every build publishes a new one.
[ ! -e "$output" ] || { echo "$output exists: choose a new release directory" >&2; exit 2; }
case "$(uname -m)" in
  aarch64)
    deb_arch=arm64
    mirror="${mirror:-http://ports.ubuntu.com/ubuntu-ports}"
    ;;
  x86_64)
    deb_arch=amd64
    mirror="${mirror:-http://archive.ubuntu.com/ubuntu}"
    ;;
  *)
    echo "no Cloud image is built for $(uname -m)" >&2
    exit 2
    ;;
esac
# The Ubuntu release of the image, 26.04; the manifest records it.
suite=resolute
# What the build creates is readable by `demi`, whatever the caller's umask.
umask 022

rm -rf "$work"
mkdir -p "$work"
# debootstrap's Ubuntu suites are one script under different names; a host
# older than the suite lacks the name.
scripts=/usr/share/debootstrap/scripts
[ -e "$scripts/$suite" ] || ln -s gutsy "$scripts/$suite"
debootstrap --arch="$deb_arch" --variant=minbase --include=apt-utils \
  "$suite" "$work" "$mirror"
cat > "$work/etc/apt/sources.list" <<SOURCES
deb $mirror $suite main universe
deb $mirror $suite-updates main universe
deb $mirror $suite-security main universe
SOURCES
in_chroot() {
  chroot "$work" /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin \
    DEBIAN_FRONTEND=noninteractive HOME=/root "$@"
}
cleanup_mounts() {
  build_status=$?
  for path in "$work/dev" "$work/sys" "$work/proc"; do
    if mountpoint -q "$path"; then
      umount "$path" || build_status=1
    fi
  done
  exit "$build_status"
}
trap cleanup_mounts EXIT
mount -t proc proc "$work/proc"
mount -t sysfs sys "$work/sys"
mount --bind /dev "$work/dev"
cp /etc/resolv.conf "$work/etc/resolv.conf"
printf '#!/bin/sh\nexit 101\n' > "$work/usr/sbin/policy-rc.d"
chmod 0755 "$work/usr/sbin/policy-rc.d"
in_chroot apt-get update
in_chroot apt-get install -y --no-install-recommends \
  $(grep -v '^#' "$here/rootfs/packages.txt") tini
in_chroot locale-gen en_US.UTF-8
in_chroot apt-get clean
rm -rf "$work/var/lib/apt/lists/"*

# The guest user, its sudo, its shell.
in_chroot groupadd -g 1000 demi
in_chroot useradd -m -u 1000 -g 1000 -s /bin/bash demi
# The runner's Host log lives on the system layer, so it outlives a stop and
# a wake (runner.md § Host log). Made inside the tree, where `demi` is a
# name: uutils' install on the build host refuses a numeric owner.
in_chroot install -d -m 0700 -o demi -g demi /var/log/demi
cp -a --no-preserve=ownership "$here/rootfs/overlay/." "$work/"
chmod 0440 "$work/etc/sudoers.d/demi"
echo demi > "$work/etc/hostname"
# Programs see the sandbox mount table.
ln -sf /proc/self/mounts "$work/etc/mtab"
# /home is the owner's image; the rootfs carries only the mount point.
rm -rf "$work/home/demi"
mkdir -p "$work/home"
umount "$work/dev" "$work/sys" "$work/proc"
trap - EXIT

# No runtime state, network configuration or machine identity of the builder.
rm -f "$work/etc/resolv.conf"
rm -rf "$work/dev/"* "$work/run/"* "$work/tmp/"*
rm -f "$work/etc/machine-id" "$work/var/lib/dbus/machine-id"
touch "$work/etc/resolv.conf"
# The second stage prints the release's base version.
"$xtask" cloud-image package --root "$work" --runners "$runners" \
  "${packages[@]}" --output "$output"
rm -rf "$work"
