#!/usr/bin/env bash
# Installs the Cloud manager in the local Lima VM
# (`docs/guides/mac-development.md`): starts or creates the VM, copies the
# manager and demi-server built for its Linux target into the bin/ of the
# manager's server release root in the VM, whose image/ the image build
# wrote, has demi-server fetch the pinned gVisor version, writes the VM's
# configuration file, and runs the host install script there. --public-url
# is the backend's URL at the Mac's address on its network, which the Cloud
# guests reach through Lima and the Mac's own runner reaches too; Lima's
# gateway address (host.lima.internal) exists only inside the VM. --dns
# names the resolvers the Clouds use in place of the VM's own, for a Mac
# whose proxy answers name lookups with addresses the Clouds may not reach.
#
# With --root DIR the configuration file and the unit are written beneath DIR
# inside the VM for review; the manager still goes into the release.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
instance=demi-machine-manager
public_url=""
slots=16
manager=""
server=""
release=""
data=/mnt/lima-demi-cloud-data/state
data_size=100GiB
dns=""
root=""
usage() {
  echo 'usage: lima-machines.sh --manager PATH --server PATH --release DIR --public-url URL' >&2
  echo '         [--dns ADDRESSES] [--data DIR] [--data-size SIZE] [--slots COUNT]' >&2
  echo '         [--root DIR]' >&2
  exit 2
}
while [ "$#" -gt 0 ]; do
  [ "$#" -ge 2 ] || usage
  case "$1" in
    --manager) manager=$2 ;;
    --server) server=$2 ;;
    --release) release=$2 ;;
    --data) data=$2 ;;
    --data-size) data_size=$2 ;;
    --public-url) public_url=$2 ;;
    --dns) dns=$2 ;;
    --slots) slots=$2 ;;
    --root) root=$2 ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
  shift 2
done
# --manager and --server are the Linux builds on this Mac; --release and
# --data are VM paths.
[ -n "$manager" ] && [ -n "$server" ] && [ -n "$release" ] && [ -n "$public_url" ] || usage
for program in "$manager" "$server"; do
  [ -f "$program" ] || { echo "no executable at $program" >&2; exit 2; }
done
manager="$(cd "$(dirname "$manager")" && pwd)/$(basename "$manager")"
server="$(cd "$(dirname "$server")" && pwd)/$(basename "$server")"
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
prefix=${root%/}
# The VM's configuration file. Without --dns the Clouds use the VM's own
# resolver, the manager's default.
settings=(
  "DEMI_BACKEND_PUBLIC_URL=$public_url"
  "DEMI_MACHINE_MANAGER_SOCKET=/run/user/$uid/demi-machine-manager.sock"
  "DEMI_MANAGED_DATA=$data"
  "DEMI_MANAGED_SLOTS=$slots"
)
if [ -n "$dns" ]; then
  settings+=("DEMI_MANAGED_DNS=$dns")
fi
limactl shell "$instance" -- sudo install -d "$prefix/opt/demi/config"
printf '%s\n' "${settings[@]}" |
  limactl shell "$instance" -- sudo tee "$prefix/opt/demi/config/demi.env" >/dev/null
# Each program is renamed into place, so a running one keeps its own file.
for program in "$manager" "$server"; do
  name=$(basename "$program")
  limactl shell "$instance" -- sudo install -D -m 0755 "$program" "$release/bin/.$name.new"
  limactl shell "$instance" -- sudo mv "$release/bin/.$name.new" "$release/bin/$name"
done
# The root the image build made holds no unit; a server release carries it,
# and so does this one from here on.
limactl shell "$instance" -- sudo install -D -m 0644 "$here/systemd/demi-machine-manager.service" "$release/systemd/demi-machine-manager.service"
limactl shell "$instance" -- sudo "$release/bin/demi-server" runtime
if [ -n "$root" ]; then
  # The VM sees this Mac's home at the same path, so the script is read there.
  limactl shell "$instance" -- bash "$here/scripts/install-managed-hosts.sh" \
    --user "$guest_user" --release "$release" --root "$root"
  exit 0
fi
# An existing instance needs a prepared Linux directory supplied through --data.
# Never reinterpret or reformat its older deployment's storage.
if [ "$data" = /mnt/lima-demi-cloud-data/state ]; then
  limactl shell "$instance" -- findmnt /mnt/lima-demi-cloud-data >/dev/null
  limactl shell "$instance" -- sudo mkdir -p "$data"
fi
limactl shell "$instance" -- sudo bash "$here/scripts/install-managed-hosts.sh" \
  --user "$guest_user" --release "$release"
echo "DEMI_MACHINE_MANAGER_SOCKET=$HOME/.lima/$instance/sock/demi-machine-manager.sock"
echo "DEMI_BACKEND_PUBLIC_URL=$public_url"
limactl shell "$instance" -- sudo journalctl -u demi-machine-manager.service -n 10 --no-pager
