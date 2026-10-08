#!/usr/bin/env bash
# Builds a Cloud image release (images.md § Build pipeline) as root on a Linux
# builder, for the builder's architecture. This script is the first stage:
# Ubuntu 26.04 from Ubuntu's container root filesystem that ubuntu.json pins,
# its packages brought up to date, the packages of packages.txt and tini, the
# guest user `demi` (uid 1000) with passwordless sudo, and the file overlay.
# The second stage is `xtask cloud-image package`, built for this builder: it
# embeds the runner and the command packages of a server release, taken from
# the release's files, and publishes the verified root archive and manifest.
#
# Usage: sudo bash rootfs/build.sh --xtask PATH --release DIR --files DIR
#          --output DIR [--work DIR] [--mirror URL] [--ca FILE]
#
# apt inside the tree runs with a cleared environment. A builder that reaches
# the mirror only through an HTTPS proxy sets https_proxy, which apt there
# then uses too. A builder whose proxy re-signs TLS passes --ca with the bundle
# that holds the proxy's authority, which apt there then trusts instead of the
# tree's own, and so does curl here; the file is gone from the tree when the
# build ends. curl, which downloads the container image, and xtask run with
# the caller's environment.
# The builder needs curl, jq, GNU tar and util-linux.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
xtask=""
release=""
files=""
output=""
work=/var/tmp/demi-cloud-root
mirror=""
ca=""
usage() {
  echo 'usage: build.sh --xtask PATH --release DIR --files DIR' >&2
  echo '         --output DIR [--work DIR] [--mirror URL] [--ca FILE]' >&2
  exit 2
}
while [ "$#" -gt 0 ]; do
  [ "$#" -ge 2 ] || usage
  case "$1" in
    --xtask) xtask=$2 ;;
    --release) release=$2 ;;
    --files) files=$2 ;;
    --output) output=$2 ;;
    --work) work=$2 ;;
    --mirror) mirror=$2 ;;
    --ca) ca=$2 ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
  shift 2
done
[ -n "$xtask" ] && [ -n "$release" ] && [ -n "$files" ] && [ -n "$output" ] || usage
[ -x "$xtask" ] || { echo "no xtask executable at $xtask" >&2; exit 2; }
[ -z "$ca" ] || [ -f "$ca" ] || { echo "no certificate bundle at $ca" >&2; exit 2; }
# A release is immutable: every build publishes a new one.
[ ! -e "$output" ] || { echo "$output exists: choose a new release directory" >&2; exit 2; }
# The image's apt names Ubuntu's own archive of its architecture; --mirror
# changes only where the build fetches from, which is the builder's network.
case "$(uname -m)" in
  aarch64)
    deb_arch=arm64
    ubuntu_archive=http://ports.ubuntu.com/ubuntu-ports
    ;;
  x86_64)
    deb_arch=amd64
    ubuntu_archive=http://archive.ubuntu.com/ubuntu
    ;;
  *)
    echo "no Cloud image is built for $(uname -m)" >&2
    exit 2
    ;;
esac
mirror="${mirror:-$ubuntu_archive}"
# The Ubuntu release of the image, 26.04; the manifest records it.
suite=resolute
# What the build creates is readable by `demi`, whatever the caller's umask.
umask 022

rm -rf "$work"
mkdir -p "$work"
# The container image of the pinned serial, checked against the pin before a
# byte of it is unpacked. The archive lies in the tree's directory until it is
# unpacked, so a failed build leaves it where the next one removes it.
pin="$here/rootfs/ubuntu.json"
url="$(jq -er --arg arch "$deb_arch" '.[$arch].url' "$pin")"
size="$(jq -er --arg arch "$deb_arch" '.[$arch].size' "$pin")"
sha256="$(jq -er --arg arch "$deb_arch" '.[$arch].sha256' "$pin")"
archive="$work/ubuntu-root.tar.gz"
curl_options=(--fail --silent --show-error --location --proto '=https')
if [ -n "$ca" ]; then
  curl_options+=(--cacert "$ca")
fi
curl "${curl_options[@]}" --output "$archive" "$url"
downloaded="$(stat -c %s "$archive")"
[ "$downloaded" = "$size" ] || {
  echo "$url has $downloaded bytes, the pin $size" >&2
  exit 1
}
echo "$sha256  $archive" | sha256sum --check --quiet - || {
  echo "$url is not the SHA-256 the pin names" >&2
  exit 1
}
tar --numeric-owner -xpzf "$archive" -C "$work"
rm "$archive"
# Writes the tree's apt sources, which name the archive at $1.
write_sources() {
  cat > "$work/etc/apt/sources.list.d/ubuntu.sources" <<SOURCES
Types: deb
URIs: $1
Suites: $suite $suite-updates $suite-security
Components: main universe
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
SOURCES
}
write_sources "$mirror"
in_chroot() {
  chroot "$work" /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin \
    DEBIAN_FRONTEND=noninteractive HOME=/root "$@"
}
# What apt inside the tree needs of the caller's network: its HTTPS proxy and
# the certificate bundle --ca names.
apt_options=()
if [ -n "${https_proxy:-}" ]; then
  apt_options+=(-o "Acquire::https::Proxy=$https_proxy")
fi
if [ -n "$ca" ]; then
  install -m 0644 "$ca" "$work/tmp/build-ca.pem"
  apt_options+=(-o Acquire::https::CaInfo=/tmp/build-ca.pem)
fi
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
# The serial's packages brought up to date, so the image carries the security
# updates published by its build, then the base's own.
in_chroot apt-get "${apt_options[@]}" update
in_chroot apt-get "${apt_options[@]}" full-upgrade -y --no-install-recommends
in_chroot apt-get "${apt_options[@]}" install -y --no-install-recommends \
  $(grep -v '^#' "$here/rootfs/packages.txt") tini
in_chroot apt-get clean
rm -rf "$work/var/lib/apt/lists/"*

# The guest user, its sudo, its shell. The container image's own user,
# `ubuntu`, holds uid and gid 1000, which are the guest user's.
in_chroot userdel --remove ubuntu
in_chroot groupadd -g 1000 demi
in_chroot useradd -m -u 1000 -g 1000 -s /bin/bash demi
# The runner's Host log and its job output live on the system layer, so they
# outlive a stop and a wake (runner.md § Host log, managed-hosts.md § Images).
# Made inside the tree, where `demi` is a name: uutils' install on the build
# host refuses a numeric owner.
in_chroot install -d -m 0700 -o demi -g demi /var/log/demi /var/lib/demi
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

# No runtime state, network configuration or machine identity of the builder,
# whose mirror included.
write_sources "$ubuntu_archive"
rm -f "$work/etc/resolv.conf"
rm -rf "$work/dev/"* "$work/run/"* "$work/tmp/"*
rm -f "$work/etc/machine-id" "$work/var/lib/dbus/machine-id"
touch "$work/etc/resolv.conf"
# The second stage prints the release's base version.
"$xtask" cloud-image package --root "$work" --release "$release" \
  --files "$files" --output "$output"
rm -rf "$work"
