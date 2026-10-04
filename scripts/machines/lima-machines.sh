#!/usr/bin/env bash
# Installs the Cloud manager in the local Lima VM
# (`docs/guides/mac-development.md`): starts or creates the VM, copies the manager built for its
# Linux target into it, and runs the host install script there. --backend-url
# is the backend's URL at the Mac's address on its network, which the Cloud
# guests reach through Lima and the Mac's own runner reaches too; Lima's
# gateway address (host.lima.internal) exists only inside the VM.
#
# With --root DIR the install script only writes the unit and configuration
# beneath DIR inside the VM, which changes nothing else there.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
instance=demi-machine-manager
backend=""
slots=16
manager=""
image=""
dns=""
data=/mnt/lima-demi-cloud-data/state
data_size=100GiB
root=""
usage() {
  echo 'usage: lima-machines.sh --manager PATH --image DIR --backend-url URL --dns ADDRESSES' >&2
  echo '         [--data DIR] [--data-size SIZE] [--slots COUNT] [--root DIR]' >&2
  exit 2
}
while [ "$#" -gt 0 ]; do
  [ "$#" -ge 2 ] || usage
  case "$1" in
    --manager) manager=$2 ;;
    --image) image=$2 ;;
    --dns) dns=$2 ;;
    --data) data=$2 ;;
    --data-size) data_size=$2 ;;
    --backend-url) backend=$2 ;;
    --slots) slots=$2 ;;
    --root) root=$2 ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
  shift 2
done
# --manager is the Linux build on this Mac; --image and --data are VM paths.
[ -n "$manager" ] && [ -n "$image" ] && [ -n "$backend" ] && [ -n "$dns" ] || usage
[ -f "$manager" ] || { echo "no manager executable at $manager" >&2; exit 2; }
manager="$(cd "$(dirname "$manager")" && pwd)/$(basename "$manager")"
case "$(limactl list --format '{{.Status}}' "$instance" 2>/dev/null)" in
  Running) ;;
  Stopped) limactl start "$instance" ;;
  *)
    limactl disk ls --format '{{.Name}}' | grep -qx demi-cloud-data || limactl disk create demi-cloud-data --size "$data_size"
    limactl start --name "$instance" "$here/lima/demi-machine-manager.yaml"
    ;;
esac
guest_user=$(limactl shell "$instance" -- id -un)
uid=$(limactl shell "$instance" -- id -u)
settings=(
  --user "$guest_user" --image "$image"
  --backend-url "$backend" --dns "$dns"
  --data "$data" --socket "/run/user/$uid/demi-machine-manager.sock" --slots "$slots"
)
if [ -n "$root" ]; then
  # The VM sees this Mac's home at the same path, so the build is read there.
  limactl shell "$instance" -- bash "$here/install-managed-hosts.sh" \
    "${settings[@]}" --manager "$manager" --root "$root"
  exit 0
fi
# An existing instance needs a prepared Linux directory supplied through --data.
# Never reinterpret or reformat its older deployment's storage.
if [ "$data" = /mnt/lima-demi-cloud-data/state ]; then
  limactl shell "$instance" -- findmnt /mnt/lima-demi-cloud-data >/dev/null
  limactl shell "$instance" -- sudo mkdir -p "$data"
fi
# The service runs a copy named by its digest, so a later build never
# replaces the executable of a running manager.
digest=$(shasum -a 256 "$manager" | awk '{print $1}')
installed="/opt/demi-machine-manager/$digest/demi-machine-manager"
limactl shell "$instance" -- sudo install -D -m 0755 "$manager" "$installed"
limactl shell "$instance" -- sudo bash "$here/install-managed-hosts.sh" \
  "${settings[@]}" --manager "$installed"
echo "DEMI_MACHINE_MANAGER_SOCKET=$HOME/.lima/$instance/sock/demi-machine-manager.sock"
echo "DEMI_BACKEND_PUBLIC_URL=$backend"
limactl shell "$instance" -- sudo journalctl -u demi-machine-manager.service -n 10 --no-pager
