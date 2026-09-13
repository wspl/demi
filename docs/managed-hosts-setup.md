# Set up managed Cloud hosts

This guide installs the machine manager for Linux or macOS development.
[Managed hosts](demi-next/managed-hosts.md) defines lifecycle, persistence, and
isolation. The backend and manager are separate processes. Build the guest kernel
and rootfs using the [guest-image instructions](../packages/guest-image/README.md)
before starting the manager.

## Configuration

| Variable | Meaning |
|---|---|
| `DEMI_MACHINES_SOCKET` | the Unix socket the manager listens on and the backend dials (required on both) |
| `DEMI_MACHINES_DATA` | the manager's state: working VM files and the image generations (default `~/.demi/machines`) |
| `DEMI_MANAGED_FIRECRACKER`, `DEMI_MANAGED_KERNEL`, `DEMI_MANAGED_ROOTFS` | the Firecracker binary and the guest-image artifacts (required) |
| `DEMI_MANAGED_LAUNCH` | `direct` (default) or `jailer`, with `DEMI_MANAGED_JAILER`, `DEMI_MANAGED_HELPER`, `DEMI_MANAGED_CHROOT_BASE`, `DEMI_MANAGED_UID_BASE` |
| `DEMI_MANAGED_VCPUS`, `DEMI_MANAGED_MEM_MIB`, `DEMI_MANAGED_SYSTEM_MIB`, `DEMI_MANAGED_HOME_MIB`, `DEMI_MANAGED_SUBNET`, `DEMI_MANAGED_SLOTS`, `DEMI_MANAGED_DNS` | guest sizing and the tap pool |

The backend needs `DEMI_MACHINES_SOCKET` and `DEMI_BACKEND_PUBLIC_URL`, the
URL guests dial.

## Install jailer mode on Linux

Run these steps on the Linux host that runs the manager. The examples use the
service account `demi-machines`, Firecracker and jailer installed as
root-owned executables at `/opt/firecracker/firecracker` and
`/opt/firecracker/jailer`, and the default chroot base `/srv/jailer`. Bash,
coreutils, sudo, e2fsprogs and KVM are required; the networking setup also
uses iproute2 and nftables. The writable disk images and chroot base must be
on the same filesystem because the script hardlinks those images into each
jail.

The machines package includes `scripts/`; no helper compilation is required.
From the package directory (`packages/machines` in a checkout), install the
launcher under a root-owned directory:

```sh
sudo install -d -o root -g root -m 0755 /usr/local/libexec/demi
sudo install -o root -g root -m 0755 scripts/firecracker-jailer.sh \
  /usr/local/libexec/demi/firecracker-jailer.sh
sudo install -d -o root -g root -m 0755 /srv/jailer
```

Use `sudo visudo -f /etc/sudoers.d/demi-firecracker` to add exactly these two
allowed operations, replacing the account name if necessary:

```sudoers
demi-machines ALL=(root) NOPASSWD: /usr/local/libexec/demi/firecracker-jailer.sh vm start *, /usr/local/libexec/demi/firecracker-jailer.sh vm kill *
```

The manager invokes this installed path directly through `sudo -n`; sudoers
must name the script, not `/bin/bash`. Keep the script and its parent
directories unwritable by the manager's account. This account is trusted to
supply executable and image paths: the helper permission is an infrastructure
privilege, not a security boundary against the manager itself.

Set the manager service environment:

```sh
DEMI_MACHINES_SOCKET=/run/demi/machines.sock
DEMI_MACHINES_DATA=/var/lib/demi-machines
DEMI_MANAGED_LAUNCH=jailer
DEMI_MANAGED_FIRECRACKER=/opt/firecracker/firecracker
DEMI_MANAGED_JAILER=/opt/firecracker/jailer
DEMI_MANAGED_HELPER=/usr/local/libexec/demi/firecracker-jailer.sh
DEMI_MANAGED_CHROOT_BASE=/srv/jailer
DEMI_MANAGED_UID_BASE=20000
DEMI_MANAGED_KERNEL=/opt/demi-guest/vmlinux
DEMI_MANAGED_ROOTFS=/opt/demi-guest/rootfs.ext4
```

Point the last two variables at the deployed guest-image artifacts. The
manager pins its own base-image copies before starting a VM. Use the same uid
base, slot count and subnet when preparing networking; for example:

```sh
sudo bash scripts/install-managed-hosts.sh \
  --user demi-machines \
  --mode jailer \
  --uid-base 20000 \
  --slots 256 \
  --subnet 172.16.0.0/16 \
  --backend-address 172.16.0.1 \
  --backend-port 3271
```

Replace the backend address and port with the guest-reachable listener. This
network script configures taps and firewall rules only; it does not install the
launcher or edit sudoers. Repeat networking setup after reboot.

Run the backend under the socket owner account and set
`DEMI_MACHINES_SOCKET=/run/demi/machines.sock`. Restart both services after
configuration. The manager socket uses owner-only access.

## Run the manager on macOS

Requirements: Lima 2.0 or later and macOS 15 on Apple silicon (nested
virtualization). Build or fetch the guest-image artifacts into
`packages/guest-image/out/aarch64/`, then, from the checkout:

```sh
packages/machines/scripts/lima-machines.sh --backend-port 3271
```

The script creates the `demi-machines` instance from the template on first
use (Ubuntu 26.04, Firecracker, e2fsprogs, nftables and Bun provisioned; the
home directory mounted read-only so the checkout is visible at the same path),
prepares the tap pool with the Mac as the backend address (printed as the
`DEMI_BACKEND_PUBLIC_URL` to use), and runs the manager in the foreground on
`/run/user/<uid>/demi-machines.sock`, which Lima forwards to
`~/.lima/demi-machines/sock/demi-machines.sock`. The manager's state lives on
the instance disk under `/var/lib/demi-machines`.

The install script adds the guest user to `kvm`. The launcher starts the
manager through `sudo -n -H -u <guest-user>` so its supplementary groups are
loaded from the guest's user database, including when Lima reuses an SSH
session opened before installation. The manager and Firecracker run as that
ordinary user. Access relies on the device's `kvm` group, not a temporary
per-user ACL that udev or logind can replace. Before starting the manager,
the launcher opens `/dev/kvm` read-write as that user and fails immediately
with a permission diagnostic if it cannot.

Configure the backend
[native command releases](demi-next/native-runtime.md#backend-deployment-configuration)
first, including `DEMI_NATIVE_CONFIG`. Then start the backend on the Mac:

```sh
DEMI_MACHINES_SOCKET=$HOME/.lima/demi-machines/sock/demi-machines.sock \
DEMI_BACKEND_PUBLIC_URL=http://192.168.5.2:3271 \
bun run --conditions development packages/backend/src/main.ts
```

