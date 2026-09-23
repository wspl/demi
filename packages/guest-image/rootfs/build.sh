#!/usr/bin/env bash
# The container root filesystem (managed-hosts.md § The shipped base): Ubuntu
# 26.04 by debootstrap, the toolchain from packages.txt, uv as one binary,
# the guest user `demi` (uid 1000) with passwordless sudo, the runner as
# /usr/bin/demi-runner, released as a verified root archive.
# Runs as root on Linux. Usage: sudo rootfs/build.sh <aarch64|x86_64>
set -euo pipefail
arch="${1:?arch}"
here="$(cd "$(dirname "$0")/.." && pwd)"
out="${DEMI_CLOUD_BUILD_DIR:-$here/out/$arch}"
image="${DEMI_CLOUD_IMAGE_OUTPUT:?set DEMI_CLOUD_IMAGE_OUTPUT to a new release directory}"
runner="${DEMI_CLOUD_RUNNER:-$out/demi-runner}"
[ -x "$runner" ] || {
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
[ "$(uname -m)" = "$arch" ] || { echo 'assemble the image on a matching Linux architecture' >&2; exit 2; }
work="${ROOTFS_WORK:-$here/out/rootfs-$arch}"
suite="${UBUNTU_SUITE:-resolute}"
uv_version="${UV_VERSION:-0.12.13}"
mirror="${UBUNTU_MIRROR:-http://ports.ubuntu.com/ubuntu-ports}"
[ "$deb_arch" = amd64 ] && mirror="${UBUNTU_MIRROR:-http://archive.ubuntu.com/ubuntu}"

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

# Install the same pinned browser archive consumed by paired-device installers.
bun --conditions development "$here/../../scripts/native/install-browser.ts" "$work" "$arch"

# The guest user, its sudo, its shell.
in_chroot groupadd -g 1000 demi
in_chroot useradd -m -u 1000 -g 1000 -s /bin/bash demi
# The runner's Host log lives on the system layer, so it outlives a stop and
# a wake (runner.md § Host log).
install -d -m 0700 -o 1000 -g 1000 "$work/var/log/demi"
cp -a --no-preserve=ownership "$here/rootfs/overlay/." "$work/"
chmod 0440 "$work/etc/sudoers.d/demi"
echo demi > "$work/etc/hostname"
# Programs see the sandbox mount table.
ln -sf /proc/self/mounts "$work/etc/mtab"

# uv: one binary from its release, checked against the published digest.
uv_dir="$out/uv-$uv_version"
uv_asset="uv-$arch-unknown-linux-gnu.tar.gz"
if [ ! -x "$uv_dir/uv" ]; then
  mkdir -p "$uv_dir"
  uv_url="https://github.com/astral-sh/uv/releases/download/$uv_version/$uv_asset"
  curl -fsSL "$uv_url" -o "$uv_dir/$uv_asset"
  curl -fsSL "$uv_url.sha256" -o "$uv_dir/$uv_asset.sha256"
  (cd "$uv_dir" && sha256sum -c "$uv_asset.sha256")
  tar -xzf "$uv_dir/$uv_asset" -C "$uv_dir" --strip-components=1
fi
install -m 0755 "$uv_dir/uv" "$uv_dir/uvx" "$work/usr/local/bin/"

# Init reaps processes; the runner supplies per-job command aliases.
install -m 0755 "$runner" "$work/usr/bin/demi-runner"
ln -s demi-runner "$work/usr/bin/demi"
# /home is the owner's image; the rootfs carries only the mount point.
rm -rf "$work/home/demi"
mkdir -p "$work/home"
umount "$work/dev" "$work/sys" "$work/proc"
trap - EXIT
rm -f "$work/etc/resolv.conf"

# Native command packages and the immutable release are packed by the shared schema.
rm -rf "$work/dev/"* "$work/run/"* "$work/tmp/"*
rm -f "$work/etc/machine-id" "$work/var/lib/dbus/machine-id"
touch "$work/etc/resolv.conf"
bun --conditions development "$here/package.ts" \
  --root "$work" --arch "$arch" --output "$image" \
  --uv-version "$uv_version" --uv-archive "$uv_dir/$uv_asset" "${@:2}"
[ "${KEEP_ROOTFS_WORK:-}" = 1 ] || rm -rf "$work"
echo "$image"
