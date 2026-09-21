#!/usr/bin/env bash
# Start the same privileged Cloud service inside the local Lima Linux VM.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"
instance=demi-machines
backend_port=3271
slots=16
image=""
dns=""
data=/mnt/lima-demi-cloud-data/state
data_size=100GiB
while [ "$#" -gt 0 ]; do
  case "$1" in
    --backend-port) backend_port=$2 ;;
    --slots) slots=$2 ;;
    --image) image=$2 ;;
    --dns) dns=$2 ;;
    --data) data=$2 ;;
    --data-size) data_size=$2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift 2
done
[ -n "$image" ] && [ -n "$dns" ] || { echo '--image and --dns are required (Linux paths and reachable IPv4 resolvers)' >&2; exit 2; }
case "$(limactl list --format '{{.Status}}' "$instance" 2>/dev/null)" in
  Running) ;;
  Stopped) limactl start "$instance" ;;
  *)
    limactl disk ls --format '{{.Name}}' | grep -qx demi-cloud-data || limactl disk create demi-cloud-data --size "$data_size"
    limactl start --name "$instance" "$here/lima/demi-machines.yaml"
    ;;
esac
backend_address=$(limactl shell "$instance" -- getent ahostsv4 host.lima.internal | awk 'NR==1 {print $1}')
[ -n "$backend_address" ] || { echo 'host.lima.internal has no IPv4 address' >&2; exit 1; }
guest_user=$(limactl shell "$instance" -- id -un)
guest_home=$(limactl shell "$instance" -- printenv HOME)
uid=$(limactl shell "$instance" -- id -u)
# An existing instance needs a prepared Linux directory supplied through --data.
# Never reinterpret or reformat its older deployment's storage.
if [ "$data" = /mnt/lima-demi-cloud-data/state ]; then
  limactl shell "$instance" -- findmnt /mnt/lima-demi-cloud-data >/dev/null
  limactl shell "$instance" -- sudo mkdir -p "$data"
fi
limactl shell "$instance" -- sudo bash "$here/scripts/install-managed-hosts.sh" \
  --user "$guest_user" --bun "$guest_home/.bun/bin/bun" \
  --manager "$root/packages/machines/src/main.ts" --image "$image" \
  --backend-url "http://$backend_address:$backend_port" --dns "$dns" \
  --data "$data" --socket "/run/user/$uid/demi-machines.sock" --slots "$slots"
echo "DEMI_MACHINES_SOCKET=$HOME/.lima/$instance/sock/demi-machines.sock"
echo "DEMI_BACKEND_PUBLIC_URL=http://$backend_address:$backend_port"
limactl shell "$instance" -- sudo journalctl -u demi-machines.service -n 10 --no-pager
