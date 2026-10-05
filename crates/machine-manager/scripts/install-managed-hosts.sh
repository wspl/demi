#!/usr/bin/env bash
# Installs the Cloud machine manager of a server release as a systemd
# service on a developer's Linux host (`setup.md` § Storage and service
# setup): it points /opt/demi/current at the release, as a server's layout
# does (`upgrades.md` § One release on a server), and installs the release's
# own unit, which runs the manager /opt/demi/current names with the
# deployment's configuration file and the release's pinned runsc. The
# release, the configuration file and the state directory on its own Linux
# filesystem must exist already; the installer reads the state directory
# from the file and writes no setting.
#
# With --root DIR the link and the unit are written beneath DIR for review,
# the configuration file is read from beneath DIR, and nothing else on the
# system changes.
set -euo pipefail
user=""
release=""
root=/
config=/etc/demi/demi.env
usage() {
  echo 'usage: install-managed-hosts.sh --user USER --release DIR [--root DIR]' >&2
  exit 2
}
while [ "$#" -gt 0 ]; do
  [ "$#" -ge 2 ] || usage
  case "$1" in
    --user) user=$2 ;;
    --release) release=$2 ;;
    --root) root=$2 ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
  shift 2
done
[ -n "$user" ] && [ -n "$release" ] || usage
# The unit takes the values verbatim: none may bring quoting, expansion or a
# systemd specifier.
for value in "$user" "$release" "$root"; do
  [[ "$value" =~ ^[a-zA-Z0-9_./:@,?=+\&-]+$ ]] || { echo "unsupported service configuration characters: $value" >&2; exit 2; }
done
for path in "$release" "$root"; do
  [[ "$path" = /* ]] || { echo "service paths must be absolute: $path" >&2; exit 2; }
done
installing=true
[ "$root" = / ] || installing=false
if $installing; then
  [ "$(id -u)" = 0 ] || { echo 'run as root' >&2; exit 2; }
fi
manager="$release/bin/demi-machine-manager"
[ -x "$manager" ] || { echo "the release has no manager: $manager" >&2; exit 2; }
[ -f "$release/image/manifest.json" ] || { echo "the release has no Cloud image: $release/image" >&2; exit 2; }
[ -x "$release/runtime/runsc" ] || { echo "the release has no runsc: assemble it with --runtime" >&2; exit 2; }
source_unit="$release/systemd/demi-machine-manager.service"
[ -f "$source_unit" ] || { echo "the release has no unit: $source_unit" >&2; exit 2; }
id "$user" >/dev/null
settings="${root%/}$config"
[ -f "$settings" ] || { echo "write the configuration file first: $settings" >&2; exit 2; }
# The state directory the manager will use: its setting in the file, or its
# default.
data=$(sed -n 's/^DEMI_MANAGED_DATA=//p' "$settings" | tail -n 1)
data=${data:-/var/lib/demi-machine-manager}
[[ "$data" = /* ]] || { echo "DEMI_MANAGED_DATA must be absolute: $data" >&2; exit 2; }
[ -d "$data" ] || { echo "prepare a Linux state directory first: $data" >&2; exit 2; }
filesystem=$(findmnt -n -o FSTYPE -T "$data")
case "$filesystem" in
  xfs|ext4|btrfs) ;;
  *) echo "unsupported Cloud storage filesystem: $filesystem" >&2; exit 2 ;;
esac
unit="${root%/}/etc/systemd/system/demi-machine-manager.service"
current="${root%/}/opt/demi/current"
mkdir -p "$(dirname "$unit")" "$(dirname "$current")"
# The unit and the link are written beside their places and renamed into
# them, so a failed install leaves the previous ones whole and no
# half-written file behind.
trap 'rm -f "$unit.new" "$current.new"' EXIT
cp "$source_unit" "$unit.new"
ln -sfn "$release" "$current.new"
chmod 0644 "$unit.new"
if ! $installing; then
  mv -T "$current.new" "$current"
  mv "$unit.new" "$unit"
  echo "wrote $unit and $current"
  exit 0
fi

getent group demi-cloud >/dev/null || groupadd --system demi-cloud
usermod -aG demi-cloud "$user"
install -d -o root -g root -m 0700 "$data"
if [ "$filesystem" = xfs ]; then
  # Clones of an image share extents; a copy-on-write hint of one block
  # keeps a write to a clone from copying more than it changes.
  xfs_io -c "cowextsize $(stat -f -c %S "$data")" "$data"
fi
# A running manager stops under the unit it was started with, so its own
# stop-post recovery runs; then the new unit takes its place.
if systemctl is-active --quiet demi-machine-manager.service; then
  systemctl stop demi-machine-manager.service
fi
mv -T "$current.new" "$current"
mv "$unit.new" "$unit"
systemctl daemon-reload
systemctl enable demi-machine-manager.service
# With Type=notify, the start returns once the manager is ready or has failed.
if ! systemctl start demi-machine-manager.service; then
  journalctl -u demi-machine-manager.service -n 50 --no-pager >&2
  exit 1
fi
