# Demi Next: Managed Hosts

Status: target architecture contract. Implementation acceptance is recorded in `progress.md`.

## Ownership

Cloud is one persistent logical managed device per user, backed by at most one
active Firecracker microVM. Conversations and workspaces reference the device;
they do not own it. The control database enforces uniqueness for managed devices
by user. Concurrent first-use requests obtain the same device and join one boot.
The device id survives shutdown, wake, crashes and system reset.

A new conversation selects Cloud without booting it. The backend allocates or
wakes the VM when files, processes or a process-backed provider require it.
Creating a Cloud workspace ensures its directory on that same device. All the
user's projects share the filesystem, installed packages, ports and resource
budget. Isolation is between users. A workspace path is only a working directory.

## Provisioning

`backend/managed` owns user-level lifecycle and admission. Every VM and disk
operation it needs is a method of the `ManagedHostProvisioner` contract in
`@demicodes/machines`: reconcile, wake, hibernate, checkpoint, volume growth,
reset and close, plus death events. The provisioner receives the user-owned
device identity and the boot arguments; it never owns a conversation, a
transcript or a user.

The implementation runs as its own service, the machine manager
(`demi-machines`, `packages/machines/src/main.ts`), and the backend holds a
`RemoteProvisioner`: the same contract, every call carried to the manager over
a Unix socket. The split is the same on every platform; only where the manager
runs differs:

- Linux: the manager runs in place beside the backend, on the host that has
  `/dev/kvm`.
- macOS: the manager runs inside a Lima instance with nested virtualization
  (`packages/machines/lima/demi-machines.yaml`), where a `/dev/kvm` exists.
  Lima forwards the manager's socket to a path on the Mac, and guests reach the
  backend on the Mac at the address the instance knows as `host.lima.internal`
  (192.168.5.2 by default; guests dial it by address, since their resolver is
  a public one). `scripts/lima-machines.sh` creates or starts the instance,
  prepares its tap pool and runs the manager on the current checkout.

```text
macOS                                  Lima instance (Linux, nested KVM)
┌──────────────┐  Unix socket          ┌───────────────────────────────────┐
│ demi-backend │ ───────────────────▶  │ demi-machines ──▶ firecracker ×N  │
│  :3271       │  ~/.lima/…/sock/…     │   tap demi0…         │ guest      │
└──────────────┘ ◀──────────────────── │   172.16.0.0/16 ◀────┘ runner     │
                  http://192.168.5.2:3271 (DEMI_BACKEND_PUBLIC_URL)
```

The wire (`packages/machines/src/protocol.ts`) is declared with zod like the
runner wire: one request line per call, `{ id, op, params }` with the op's
parameters, answered by `{ type: 'ok', id, result }` or
`{ type: 'error', id, message }`; the manager sends `{ type: 'death',
deviceId }` on its own when a VM exits. Frames are newline-delimited JSON. The
manager listens on `DEMI_MACHINES_SOCKET` only, with the socket file readable
by its owner alone; it never listens on TCP and has no authentication. Whoever
can open the socket file is the backend. The backend dials the same path
through `DEMI_MACHINES_SOCKET`; unset, the backend has no managed hosts.

The manager reconciles on its own start (VMs left by an earlier manager are
stopped and their disks saved). The backend's `reconcile` on its start brings
every machine to off with disks saved, so the lifecycle's view (everything off
until woken) holds whichever process restarted. The backend's `close` does the
same and disconnects; the manager keeps running.

Firecracker is driven through its Unix-socket API. The manager runs Firecracker
and image utilities as the infrastructure adapters that require processes.
Two launch modes are supported:

- `direct`: an unprivileged Firecracker process with KVM and its seccomp filter,
  for self-hosting, development and trusted deployments.
- `jailer`: `packages/machines/scripts/firecracker-jailer.sh` prepares image
  paths and filesystem permissions, launches jailer and monitors Firecracker's
  recorded PID. Jailer creates the chroot, namespaces and cgroup and drops to
  the slot's uid. The script accepts `vm start` and `vm kill` with validated
  arguments; it does not implement isolation, communicate with the runner or
  carry business logic. The manager communicates with Firecracker through its
  API socket.

The guest pipeline builds a minimal Linux kernel and a read-only developer
rootfs containing bash, coreutils, compilers and the packed txiki.js runner.
The manager requires Linux with `/dev/kvm`; filesystem persistence does not
require keeping a VM running. The install script prepares the tap pool,
forwarding and nftables policy. The backend URL is reachable; other private and
link-local destinations are blocked, public egress is allowed and inbound
connections are not exposed. No host directories are mounted into a guest.

### The manager's environment

| Variable | Meaning |
|---|---|
| `DEMI_MACHINES_SOCKET` | the Unix socket the manager listens on and the backend dials (required on both) |
| `DEMI_MACHINES_DATA` | the manager's state: working VM files and the image generations (default `~/.demi/machines`) |
| `DEMI_MANAGED_FIRECRACKER`, `DEMI_MANAGED_KERNEL`, `DEMI_MANAGED_ROOTFS` | the Firecracker binary and the guest-image artifacts (required) |
| `DEMI_MANAGED_LAUNCH` | `direct` (default) or `jailer`, with `DEMI_MANAGED_JAILER`, `DEMI_MANAGED_HELPER`, `DEMI_MANAGED_CHROOT_BASE`, `DEMI_MANAGED_UID_BASE` |
| `DEMI_MANAGED_VCPUS`, `DEMI_MANAGED_MEM_MIB`, `DEMI_MANAGED_SYSTEM_MIB`, `DEMI_MANAGED_HOME_MIB`, `DEMI_MANAGED_SUBNET`, `DEMI_MANAGED_SLOTS`, `DEMI_MANAGED_DNS` | guest sizing and the tap pool |

The backend needs `DEMI_MACHINES_SOCKET` and `DEMI_BACKEND_PUBLIC_URL`, the
URL guests dial.

### Installing jailer mode

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
launcher or edit sudoers. Repeat networking setup after reboot. Give the
backend service `DEMI_MACHINES_SOCKET=/run/demi/machines.sock` and the socket
file's owner or group, then restart both services.

### The manager on macOS

Requirements: Lima 2.0 or later and macOS 15 on Apple silicon (nested
virtualization). Build or fetch the guest-image artifacts into
`packages/guest-image/out/aarch64/`, then, from the checkout:

```sh
packages/machines/scripts/lima-machines.sh --backend-port 3271
```

The script creates the `demi-machines` instance from the template on first
use (Ubuntu 24.04, Firecracker, e2fsprogs, nftables and Bun provisioned; the
home directory mounted read-only so the checkout is visible at the same path),
prepares the tap pool with the Mac as the backend address (printed as the
`DEMI_BACKEND_PUBLIC_URL` to use), and runs the manager in the foreground on
`/run/user/<uid>/demi-machines.sock`, which Lima forwards to
`~/.lima/demi-machines/sock/demi-machines.sock`. The manager's state lives on
the instance disk under `/var/lib/demi-machines`. Start the backend on the Mac
with:

```sh
DEMI_MACHINES_SOCKET=$HOME/.lima/demi-machines/sock/demi-machines.sock \
DEMI_BACKEND_PUBLIC_URL=http://192.168.5.2:3271 \
bun run --conditions development packages/backend/src/main.ts
```

## Images

A Cloud disk generation contains three references:

| Layer | Purpose | Ordinary shutdown | System reset |
|---|---|---|---|
| Versioned read-only base | shipped OS, developer tools and runner | same version | selected shipped version |
| Writable system ext4 image | persistent overlay upper/work for `/`, including `/etc`, `/usr` changes and `/var` | retained | replaced with an empty layer |
| Writable home ext4 image | `/home`, including `/home/demi` projects, session directories and user tools | retained | retained |

The base version and writable system layer are a pair. Wake never silently
changes the base beneath an existing layer. Shipping a new base makes it
available for reset and new users; it does not replace existing installations.
The guest mounts the writable system disk before assembling the overlay and
pivoting root, then mounts home separately. `/run` and `/tmp` are tmpfs.
Runner sockets, active job state, manifest cache and command output use
`/run/demi`; the backend republishes command modules after connection.

System-level installs, including `sudo apt install`, survive shutdown and wake.
User-level tools and configuration in home also persist. `/tmp` and processes
do not. Growth and quotas apply independently to system and home volumes;
initial nominal size is not a user's storage quota.

## Persistence

`storage.md` defines the machine-image store and its generation manifest. A
checkpoint captures system and home at one VM pause point and publishes one
manifest only after both images are durable. A failed save retains the prior
committed generation and the newer working files for retry. Wake must not choose
a stale stored generation while unsaved working files remain.

The runner may sync filesystems before shutdown. The provisioner must also be
able to stop and preserve a broken guest without runner cooperation. A stopped
ext4 image may require journal replay; this is filesystem crash consistency,
not a guarantee of application-level transactions across files or volumes.
Skipping a checkpoint requires evidence that neither writable volume changed.

On backend restart, reconciliation fences and terminates prior VM processes
before saving their working disks or allowing another boot. A writable disk
never has two active VM writers. Checkpoint, growth, shutdown, reset and boot
are serialized per managed device. Scaled deployment needs worker fencing in
addition to routing affinity (`execution-coordination.md`).

## Lifecycle

```text
unallocated → off → booting → running → saving → off
                               │
                               ├── checkpoint → running
                               └── resetting → off → booting
```

The user has a stable Cloud selection even before allocation. Lifecycle state
is reported separately from device identity. Errors expose the failed operation
and permit an explicit retry without creating another logical device.

- Idle reclamation requires no admitted machine operations, no active turns
  using Cloud across any of the user's conversation trees, and no running jobs
  for the configured idle window. Attachments by themselves do not count.
- A separate configured maximum unattended-job lifetime bounds forgotten
  background services. Before applying it, block admission and recheck active
  turns and operation leases. Terminate affected jobs with an explicit result.
- Hibernate saves both writable disks and releases the VM. Wake rotates the
  device token and boots the pinned disk generation. No memory snapshot is used.
- Crash-loop protection stops repeated automatic boots and exposes recovery
  actions. A system reset remains available while boot is failing.
- Archiving conversations and deleting project metadata never destroys Cloud.
  Destruction of a user's Cloud data belongs to explicit user/account data
  deletion, with its own retention policy.

Resource limits are per user: vCPU, guest memory, system quota and home quota.
The scheduler also enforces total host capacity and admission for concurrent
boots. One machine per user is a uniqueness invariant, not a configurable
host-count limit. The initial capacity profile is 2 vCPU and 2 GiB guest RAM;
measured idle memory, peak memory, wake latency and disk I/O determine deployment
capacity. Published hypervisor overhead is not an end-to-end product benchmark.

## System reset

The authenticated owner can request a reset outside the guest, including when
the guest cannot boot. The shared UI states that all Cloud tasks stop, system packages and
configuration are replaced, and `/home` is retained. It must not promise that
retaining `.bashrc` or user-installed tools repairs defects inside home.

The backend records a reset operation with a unique id, target base version,
phase and error. The machine-image manifest identifies the committed generation. It closes machine admission, interrupts affected active turns and ends all jobs
using the device across conversations, then stops the guest and preserves its
latest home image. Pending machine operations receive an interruption result;
they are not replayed against the rebuilt system. It
creates a clean system image and atomically publishes a generation containing
that image and the preserved home. It then boots and reports readiness. Device
identity, project records and conversation history remain unchanged. The backend
advances execution context for every conversation bound to the device; each node
observes the reset and invalidation of old jobs/output paths before new inference.

Before generation publication, recovery retains the prior generation and retries
or reports failure; after publication it uses the new generation and may retry
boot. New jobs are not admitted until recovery establishes the committed state.
A failed home save must not replace the generation or discard that working home.
Reset never relies on commands running inside the damaged system.

The lifecycle state and reset phase are separate fields. The UI maps `off` to
sleeping, `booting` to starting and `running` to ready; `resetting` carries the
operation's stopping/saving/rebuilding/booting phase. A failure retains the
operation id, error and committed generation so retry has an explicit subject.

## Joining

Provision and wake mint a token whose hash is held by the backend. The guest
receives `demi.backend`, `demi.token`, `demi.ip`, `demi.gw` and `demi.dns` through
the kernel command line. The runner holds the token in memory, never writes it
to the persistent disks, and authenticates directly. A managed runner without a
token is rejected; it never enters device pairing. Each token has one live
connection. User ownership is checked on every control-plane operation.

The same TypeScript runner on txiki.js serves paired devices and Cloud. On Cloud
it is PID 1, mounts filesystems, configures networking, reaps children and runs
filesystem operations and jobs as uid 1000 (`demi`, `/home/demi`, passwordless sudo). The VM, not that uid,
is the user security boundary. Backend lifecycle remains reachable if the guest
or its runner fails.

## Verification

Scripted-provider tests cover concurrent first use, multiple projects on one
machine, cross-project reads, user isolation, all-tree idle admission, wake,
crash loops and reset concurrency. Fake provisioners exercise failures before
and after generation publication. Linux/KVM smoke tests cover both launch modes,
system and home persistence, reset with retained home, broken-guest reset,
volume growth and recovery after interrupted saves. Measure command-ready
latency and actual host memory separately from configured guest RAM. Never call
real models in tests.
