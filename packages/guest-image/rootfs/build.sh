#!/usr/bin/env bash
# The shared read-only rootfs (managed-hosts.md § Images): Ubuntu 24.04 by
# debootstrap, the system toolchain from packages.txt, the guest user `demi`
# (uid 1000) with passwordless sudo, the
# runner as /demi-runner and the native client as /usr/bin/demi, packed with
# `mke2fs -d`.
# Runs as root on Linux. Usage: sudo rootfs/build.sh <aarch64|x86_64>
set -euo pipefail
arch="${1:?arch}"
here="$(cd "$(dirname "$0")/.." && pwd)"
out="$here/out/$arch"
runner="$out/demi-runner"
client="$out/demi"
[ -x "$runner" ] && [ -x "$client" ] || {
  echo "build the runner first: runner/build.sh $arch" >&2
  exit 2
}
case "$arch" in
  aarch64)
    deb_arch=arm64
    ;;
  x86_64)
    deb_arch=amd64
    ;;
  *)
    echo "unknown arch $arch" >&2
    exit 2
    ;;
esac
work="${ROOTFS_WORK:-$here/out/rootfs-$arch}"
suite="${UBUNTU_SUITE:-noble}"
mirror="${UBUNTU_MIRROR:-http://ports.ubuntu.com/ubuntu-ports}"
[ "$deb_arch" = amd64 ] && mirror="${UBUNTU_MIRROR:-http://archive.ubuntu.com/ubuntu}"
size="${ROOTFS_SIZE:-6G}"

rm -rf "$work"
mkdir -p "$work"
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
mount -t proc proc "$work/proc"
mount -t sysfs sys "$work/sys"
mount --bind /dev "$work/dev"
trap 'umount -l "$work/dev" "$work/sys" "$work/proc" 2>/dev/null || true' EXIT
cp /etc/resolv.conf "$work/etc/resolv.conf"
in_chroot apt-get update
in_chroot apt-get install -y --no-install-recommends \
  $(grep -v '^#' "$here/rootfs/packages.txt")
in_chroot locale-gen en_US.UTF-8 || true
in_chroot apt-get clean
rm -rf "$work/var/lib/apt/lists/"*

# The guest user, its sudo, its shell.
in_chroot groupadd -g 1000 demi
in_chroot useradd -m -u 1000 -g 1000 -s /bin/bash demi
cp -a --no-preserve=ownership "$here/rootfs/overlay/." "$work/"
chmod 0440 "$work/etc/sudoers.d/demi"
echo demi > "$work/etc/hostname"
# e2fsprogs uses the mount table to select online resizing. PID 1 owns mounts.
ln -sf /proc/self/mounts "$work/etc/mtab"

# The runner owns init; jobs invoke the separate native command client.
install -m 0755 "$runner" "$work/demi-runner"
install -m 0755 "$client" "$work/usr/bin/demi"
# /home is the owner's image; the rootfs carries only the mount point.
rm -rf "$work/home/demi"
mkdir -p "$work/home"
umount -l "$work/dev" "$work/sys" "$work/proc"
trap - EXIT
rm -f "$work/etc/resolv.conf"

rm -f "$out/rootfs.ext4"
mke2fs -q -t ext4 -F -L rootfs -d "$work" "$out/rootfs.ext4" "$size"
e2fsck -fy "$out/rootfs.ext4" >/dev/null || true
resize2fs -M "$out/rootfs.ext4"
# The work tree goes: a Debian tree carries symlink loops (usr/bin/X11 -> .)
# that recursive scanners never leave.
[ "${KEEP_ROOTFS_WORK:-}" = 1 ] || rm -rf "$work"
echo "$out/rootfs.ext4"
