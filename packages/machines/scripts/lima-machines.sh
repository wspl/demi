#!/usr/bin/env bash
# The machine manager on macOS: create or start the `demi-machines` Lima
# instance (`lima/demi-machines.yaml`), prepare its tap pool and egress rules,
# and run the manager inside it in the foreground, on the checkout this
# script belongs to. The backend on the Mac then dials
# `~/.lima/demi-machines/sock/demi-machines.sock` (`DEMI_MACHINES_SOCKET`)
# and tells guests to dial it back at the address the instance knows as
# `host.lima.internal`, printed below (`DEMI_BACKEND_PUBLIC_URL`).
#
#   scripts/lima-machines.sh [--backend-port 3271] [--slots 16]
#                            [--kernel <path>] [--rootfs <path>]
#
# The kernel and rootfs paths are as seen inside the instance; they default to
# the checkout's `packages/guest-image/out/<arch>/` on the home mount.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$here/../.." && pwd)"
instance=demi-machines
backend_port=3271
slots=16
kernel=""
rootfs=""
while [ $# -gt 0 ]; do
  case "$1" in
    --backend-port)
      backend_port=$2
      shift 2
      ;;
    --slots)
      slots=$2
      shift 2
      ;;
    --kernel)
      kernel=$2
      shift 2
      ;;
    --rootfs)
      rootfs=$2
      shift 2
      ;;
    *)
      echo "unknown argument $1" >&2
      exit 2
      ;;
  esac
done
arch=$(uname -m)
[ "$arch" = arm64 ] && arch=aarch64
[ -n "$kernel" ] || kernel="$root/packages/guest-image/out/$arch/vmlinux"
[ -n "$rootfs" ] || rootfs="$root/packages/guest-image/out/$arch/rootfs.ext4"

case "$(limactl list --format '{{.Status}}' "$instance" 2>/dev/null)" in
  Running)
    ;;
  Stopped)
    limactl start "$instance"
    ;;
  *)
    limactl start --name "$instance" "$here/lima/demi-machines.yaml"
    ;;
esac

# host.lima.internal, as the instance sees the Mac.
backend_address=$(limactl shell "$instance" -- getent hosts host.lima.internal | awk '{print $1; exit}')
[ -n "$backend_address" ] || {
  echo "host.lima.internal does not resolve inside $instance" >&2
  exit 1
}
# Idempotent; rerun on every start because taps do not survive a reboot.
guest_user=$(limactl shell "$instance" -- id -un)
limactl shell "$instance" -- sudo -n bash "$here/scripts/install-managed-hosts.sh" \
  --user "$guest_user" --mode direct \
  --backend-address "$backend_address" --backend-port "$backend_port" \
  --slots "$slots"
uid=$(limactl shell "$instance" -- id -u)
echo "backend: DEMI_MACHINES_SOCKET=$HOME/.lima/$instance/sock/demi-machines.sock DEMI_BACKEND_PUBLIC_URL=http://$backend_address:$backend_port"
# Lima reuses an SSH session with stale supplementary groups. sudo refreshes
# them from the guest's user database while keeping the manager unprivileged.
# Device ACLs are not durable: udev/logind can replace them.
exec limactl shell "$instance" -- sudo -n -H -u "$guest_user" env \
  "DEMI_MACHINES_SOCKET=/run/user/$uid/demi-machines.sock" \
  DEMI_MACHINES_DATA=/var/lib/demi-machines \
  DEMI_MANAGED_FIRECRACKER=/usr/local/bin/firecracker \
  "DEMI_MANAGED_KERNEL=$kernel" \
  "DEMI_MANAGED_ROOTFS=$rootfs" \
  "DEMI_MANAGED_SLOTS=$slots" \
  bash -c '
    set -euo pipefail
    if ! (exec 3<>/dev/kvm); then
      echo "demi-machines: cannot open /dev/kvm as $(id -un); check the guest kvm group and device permissions" >&2
      exit 1
    fi
    exec "$HOME/.bun/bin/bun" run --conditions development "$0"
  ' \
  "$root/packages/machines/src/main.ts"
