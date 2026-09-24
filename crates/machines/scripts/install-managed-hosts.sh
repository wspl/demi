#!/usr/bin/env bash
# Installs the Cloud machine manager as a systemd service (`setup.md` §
# Storage and service setup): the pinned runsc, the service's group, its
# configuration and its unit. The manager executable, the image release and
# the state directory on its own Linux filesystem must exist already.
#
# With --root DIR the unit and configuration are written beneath DIR for
# review, and nothing else on the system changes.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
user=""
manager=""
image=""
backend=""
dns=""
data=/var/lib/demi-machines
socket=/run/demi-cloud/machines.sock
slots=256
root=/
usage() {
  echo 'usage: install-managed-hosts.sh --user USER --manager PATH --image DIR --backend-url URL --dns ADDRESSES' >&2
  echo '         [--data DIR] [--socket PATH] [--slots COUNT] [--root DIR]' >&2
  exit 2
}
while [ "$#" -gt 0 ]; do
  [ "$#" -ge 2 ] || usage
  case "$1" in
    --user) user=$2 ;;
    --manager) manager=$2 ;;
    --image) image=$2 ;;
    --backend-url) backend=$2 ;;
    --dns) dns=$2 ;;
    --data) data=$2 ;;
    --socket) socket=$2 ;;
    --slots) slots=$2 ;;
    --root) root=$2 ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
  shift 2
done
[ -n "$user" ] && [ -n "$manager" ] && [ -n "$image" ] && [ -n "$backend" ] && [ -n "$dns" ] || usage
# The unit and the environment file take the values verbatim: none may bring
# quoting, expansion or a systemd specifier.
for value in "$user" "$manager" "$image" "$backend" "$dns" "$data" "$socket" "$slots" "$root"; do
  [[ "$value" =~ ^[a-zA-Z0-9_./:@,?=+\&-]+$ ]] || { echo "unsupported service configuration characters: $value" >&2; exit 2; }
done
for path in "$manager" "$image" "$data" "$socket" "$root"; do
  [[ "$path" = /* ]] || { echo "service paths must be absolute: $path" >&2; exit 2; }
done
installing=true
[ "$root" = / ] || installing=false
if $installing; then
  [ "$(id -u)" = 0 ] || { echo 'run as root' >&2; exit 2; }
fi
[ -x "$manager" ] || { echo "the manager is not an executable: $manager" >&2; exit 2; }
[ -f "$image/manifest.json" ] || { echo "no Cloud image release at $image" >&2; exit 2; }
id "$user" >/dev/null
[ -d "$data" ] || { echo "prepare a Linux state directory first: $data" >&2; exit 2; }
filesystem=$(findmnt -n -o FSTYPE -T "$data")
case "$filesystem" in
  xfs|ext4|btrfs) ;;
  *) echo "unsupported Cloud storage filesystem: $filesystem" >&2; exit 2 ;;
esac
if $installing; then
  runsc=$(bash "$here/install-runsc.sh")
else
  runsc=$(bash "$here/install-runsc.sh" --path)
fi

unit="${root%/}/etc/systemd/system/demi-machines.service"
config="${root%/}/etc/demi-machines/manager.env"
mkdir -p "$(dirname "$unit")" "$(dirname "$config")"
# Each file is written beside its place and renamed into it, so a failed
# install leaves the previous one whole and no half-written file behind.
trap 'rm -f "$config.new" "$unit.new"' EXIT
cat > "$config.new" <<CONFIG
DEMI_MACHINES_SOCKET=$socket
DEMI_MACHINES_DATA=$data
DEMI_MANAGED_RUNSC=$runsc
DEMI_MANAGED_IMAGE=$image
DEMI_MANAGED_BACKEND_URL=$backend
DEMI_MANAGED_DNS=$dns
DEMI_MANAGED_SLOTS=$slots
CONFIG
# Type=notify: the manager reports readiness once it has recovered, installed
# its network policy, imported its base and opened its socket; none of that,
# nor a stop's drain and its recovery, has a deadline. KillMode=mixed signals
# the manager alone, which stops its own children while it drains.
cat > "$unit.new" <<UNIT
[Unit]
Description=Demi Cloud machine manager
After=network-online.target
Wants=network-online.target
RequiresMountsFor=$data $image $manager

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
EnvironmentFile=/etc/demi-machines/manager.env
ExecStart=$manager
ExecStopPost=$manager --recover

[Install]
WantedBy=multi-user.target
UNIT
chmod 0644 "$config.new" "$unit.new"
if ! $installing; then
  mv "$config.new" "$config"
  mv "$unit.new" "$unit"
  echo "wrote $unit and $config"
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
if systemctl is-active --quiet demi-machines.service; then
  systemctl stop demi-machines.service
fi
mv "$config.new" "$config"
mv "$unit.new" "$unit"
systemctl daemon-reload
systemctl enable demi-machines.service
# With Type=notify, the start returns once the manager is ready or has failed.
if ! systemctl start demi-machines.service; then
  journalctl -u demi-machines.service -n 50 --no-pager >&2
  exit 1
fi
