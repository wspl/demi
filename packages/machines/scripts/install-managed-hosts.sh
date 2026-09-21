#!/usr/bin/env bash
# Install the privileged Cloud manager service; storage must already exist.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
user=""
bun=""
manager=""
image=""
backend=""
dns=""
data=/var/lib/demi-machines
socket=/run/demi-cloud/machines.sock
slots=256
while [ "$#" -gt 0 ]; do
  case "$1" in
    --user) user=$2 ;;
    --bun) bun=$2 ;;
    --manager) manager=$2 ;;
    --image) image=$2 ;;
    --backend-url) backend=$2 ;;
    --dns) dns=$2 ;;
    --data) data=$2 ;;
    --socket) socket=$2 ;;
    --slots) slots=$2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift 2
done
[ "$(id -u)" = 0 ] || { echo 'run as root' >&2; exit 2; }
[ -n "$user" ] && [ -n "$bun" ] && [ -n "$manager" ] && [ -n "$image" ] && [ -n "$backend" ] && [ -n "$dns" ] || {
  echo '--user --bun --manager --image --backend-url and --dns are required' >&2
  exit 2
}
# Unit values must not introduce quoting, expansion, or systemd specifiers.
for value in "$user" "$bun" "$manager" "$image" "$backend" "$dns" "$data" "$socket" "$slots"; do
  [[ "$value" =~ ^[a-zA-Z0-9_./:@,?=+\&-]+$ ]] || { echo 'unsupported service configuration characters' >&2; exit 2; }
done
for path in "$bun" "$manager" "$image" "$data" "$socket"; do
  [[ "$path" = /* ]] || { echo 'service paths must be absolute' >&2; exit 2; }
done
[ -x "$bun" ] && [ -f "$manager" ] && [ -f "$image/manifest.json" ]
id "$user" >/dev/null
[ -d "$data" ] || { echo "prepare a Linux state directory first: $data" >&2; exit 2; }
filesystem=$(findmnt -n -o FSTYPE -T "$data")
case "$filesystem" in
  xfs) xfs_io -c 'cowextsize 4096' "$data" ;;
  ext4|btrfs) ;;
  *) echo "unsupported Cloud storage filesystem: $filesystem" >&2; exit 2 ;;
esac
BUN="$bun" bash "$here/install-runsc.sh"
runsc=$("$bun" -e 'const r = await Bun.file(process.argv[1]).json(); console.log("/opt/gvisor/" + (process.arch === "arm64" ? r.arm64Version.replace(/^release-/, "") : r.upstream) + "/runsc")' "$here/../runtime/release.json")
getent group demi-cloud >/dev/null || groupadd --system demi-cloud
usermod -aG demi-cloud "$user"
install -d -o root -g root -m 0700 "$data"
install -d -o root -g demi-cloud -m 0750 /run/demi-cloud
cat > /etc/systemd/system/demi-machines.service <<UNIT
[Unit]
Description=Demi gVisor Cloud manager
After=network-online.target
Wants=network-online.target
RequiresMountsFor=$data $image $manager $bun
[Service]
Type=simple
User=root
Group=demi-cloud
PrivateMounts=yes
KillMode=control-group
TimeoutStopSec=180
Restart=on-failure
RestartSec=3
UMask=0077
RuntimeDirectory=demi-cloud
RuntimeDirectoryMode=0750
Environment=DEMI_MACHINES_SOCKET=$socket
Environment=DEMI_MACHINES_DATA=$data
Environment=DEMI_MANAGED_RUNSC=$runsc
Environment=DEMI_MANAGED_IMAGE=$image
Environment=DEMI_MANAGED_BACKEND_URL=$backend
Environment=DEMI_MANAGED_DNS=$dns
Environment=DEMI_MANAGED_SLOTS=$slots
ExecStart=$bun --conditions development $manager
ExecStopPost=$bun --conditions development $manager --recover
[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable demi-machines.service
systemctl restart demi-machines.service
