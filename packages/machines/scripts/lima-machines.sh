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
limactl shell "$instance" -- sudo -n bash "$here/scripts/install-managed-hosts.sh" \
  --user "$(id -un)" --mode direct \
  --backend-address "$backend_address" --backend-port "$backend_port" \
  --slots "$slots"
# The kvm group the install script grants takes effect at the next login,
# and Lima multiplexes its shells over one ssh session; an ACL applies now.
limactl shell "$instance" -- sudo -n setfacl -m "u:$(id -un):rw" /dev/kvm

uid=$(limactl shell "$instance" -- id -u)
echo "backend: DEMI_MACHINES_SOCKET=$HOME/.lima/$instance/sock/demi-machines.sock DEMI_BACKEND_PUBLIC_URL=http://$backend_address:$backend_port"
exec limactl shell "$instance" -- env \
  "DEMI_MACHINES_SOCKET=/run/user/$uid/demi-machines.sock" \
  DEMI_MACHINES_DATA=/var/lib/demi-machines \
  DEMI_MANAGED_FIRECRACKER=/usr/local/bin/firecracker \
  "DEMI_MANAGED_KERNEL=$kernel" \
  "DEMI_MANAGED_ROOTFS=$rootfs" \
  "DEMI_MANAGED_SLOTS=$slots" \
  bash -c 'exec "$HOME/.bun/bin/bun" run --conditions development "$0"' \
  "$root/packages/machines/src/main.ts"
