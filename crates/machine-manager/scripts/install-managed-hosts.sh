#!/usr/bin/env bash
# Installs the Cloud machine manager of a server release as a systemd
# service (`setup.md` § Storage and service setup): the pinned runsc, the
# service's group and its unit, which runs the release's manager with the
# deployment's configuration file. The release, the configuration file and
# the state directory on its own Linux filesystem must exist already; the
# installer reads the state directory from the file and writes no setting.
#
# With --root DIR the unit is written beneath DIR for review, the
# configuration file is read from beneath DIR, and nothing else on the
# system changes.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
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
  # The manager's earlier name: two managers would share the socket, the
  # network names and the nftables table (`setup.md` § Upgrading from
  # demi-machines).
  if [ -n "$(systemctl list-unit-files --no-legend demi-machines.service)" ]; then
    echo 'demi-machines.service is installed: stop and disable it first (setup.md § Upgrading from demi-machines)' >&2
    exit 2
  fi
fi
manager="$release/bin/demi-machine-manager"
[ -x "$manager" ] || { echo "the release has no manager: $manager" >&2; exit 2; }
[ -f "$release/image/manifest.json" ] || { echo "the release has no Cloud image: $release/image" >&2; exit 2; }
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
if $installing; then
  bash "$here/install-runsc.sh" >/dev/null
fi

unit="${root%/}/etc/systemd/system/demi-machine-manager.service"
mkdir -p "$(dirname "$unit")"
# The unit is written beside its place and renamed into it, so a failed
# install leaves the previous one whole and no half-written file behind.
trap 'rm -f "$unit.new"' EXIT
# Type=notify: the manager reports readiness once it has recovered, installed
# its network policy, imported its base and opened its socket; none of that,
# nor a stop's drain and its recovery, has a deadline. KillMode=mixed signals
# the manager alone, which stops its own children while it drains. The
# manager finds its release from where its executable lies.
cat > "$unit.new" <<UNIT
[Unit]
Description=Demi Cloud machine manager
After=network-online.target
Wants=network-online.target
RequiresMountsFor=$data $release

[Service]
Type=notify
User=root
Group=demi-cloud
PrivateMounts=yes
UMask=0077
KillMode=mixed
TimeoutStartSec=infinity
TimeoutStopSec=infinity
Restart=on-failure
RestartSec=3
RuntimeDirectory=demi-cloud
RuntimeDirectoryMode=0750
EnvironmentFile=$config
ExecStart=$manager
ExecStopPost=$manager --recover

[Install]
WantedBy=multi-user.target
UNIT
chmod 0644 "$unit.new"
if ! $installing; then
  mv "$unit.new" "$unit"
  echo "wrote $unit"
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
mv "$unit.new" "$unit"
systemctl daemon-reload
systemctl enable demi-machine-manager.service
# With Type=notify, the start returns once the manager is ready or has failed.
if ! systemctl start demi-machine-manager.service; then
  journalctl -u demi-machine-manager.service -n 50 --no-pager >&2
  exit 1
fi
