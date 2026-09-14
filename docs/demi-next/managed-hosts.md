# Managed Cloud hosts

Cloud is one persistent logical device per user, backed by at most one active
Firecracker microVM. Conversations and workspaces reference that device. They
share its files, installed packages, ports, and resource budget. Isolation is
between users; a workspace path is a starting directory, not a permission boundary.

This document defines provisioning, disk persistence, and reset. Target selection
and shared-device admission follow the
[conversation execution contract](sessions-and-targets.md). Installation commands
and environment settings are in [managed-host setup](../managed-hosts-setup.md).
[Resource lifecycle coordination](resource-lifecycle.md) defines the shared
scheduling/admission mechanism; this document owns Cloud policy and
VM/disk outcomes.

## Provisioning

The backend's managed-host adapter owns Cloud policy and durable transitions;
the shared backend lifecycle coordinator owns admission and idle scheduling.
The machine manager owns VM and disk operations through `ManagedHostProvisioner`:
reconcile, wake, hibernate, checkpoint, growth, reset, close, and VM-death notifications.
The manager has no conversation or transcript state.

The diagram shows control requests between processes. Each arrow names its
transport; runner connections to the backend are separate from this control path.

```text
+-------------------+
| Backend process   |
| Cloud policy      |
+-------------------+
          |
          | Provisioner requests / Unix socket
          v
+-------------------+
| Machine manager   |
| VM and disk owner |
+-------------------+
          |
          | Firecracker API / Unix socket
          v
+-------------------+
| Firecracker       |
| One user's guest  |
+-------------------+
```

On Linux the manager runs on a machine with `/dev/kvm`. On macOS the repository's
Lima template runs the same manager inside Linux with nested virtualization.
Lima forwards the manager socket to the Mac. Guests connect separately to the
backend's configured guest-reachable URL.

The manager uses newline-delimited JSON over an owner-restricted Unix socket.
Requests contain `{ id, op, params }`; replies contain an `ok` result or error for
that id. Unsolicited death events identify the exited device. The explicit schemas
live in `packages/machines/src/protocol.ts`. Socket access is the authorization
boundary; the manager provides neither a TCP listener nor an application login.

The manager reconciles leftover VMs before serving requests. Backend startup also
reconciles machines to an off state with saved disks. Closing the backend saves
and disconnects its machines without terminating the manager service.

## Images

A committed disk generation names one base version and two writable images:

| Layer | Contents | Hibernate and wake | System reset |
| --- | --- | --- | --- |
| Read-only base | Shipped OS, tools, and runner | Keep pinned version | Select reset base version |
| System ext4 image | Overlay changes to `/`, including `/etc`, `/usr`, and `/var` | Preserve | Replace with an empty layer |
| Home ext4 image | `/home`, including projects and user tools | Preserve | Preserve |

The base and system layer stay paired. Deploying a new base makes it available
for new users and resets; it does not change an existing machine's base on wake.
Temporary `/run` and `/tmp` contents and running processes do not survive shutdown.
Runner contexts, caches, and full command output under `/run/demi` are temporary.

For example, after `sudo apt install` and writing `/home/demi/project/result.txt`,
hibernate/wake preserves both the package and file. Reset removes the system
package and preserves the file. Reset also preserves user configuration under
home, including any broken `.bashrc`; it cannot promise to repair that state.

### The shipped base

The image build selects Ubuntu 26.04 with a minimal package base. The Rust runner
is `/demi-runner`, also linked as `/usr/bin/demi-runner`, and runs guest
initialization as PID 1. It mounts temporary storage and POSIX shared memory at
`/tmp` and `/dev/shm`; both disappear when the guest stops. Jobs run as `demi`,
UID 1000, with passwordless sudo.
The VM is the isolation boundary.

Preinstalled tools are system packages or standalone binaries. The exact package
list lives in [packages.txt](../../packages/guest-image/rootfs/packages.txt);
[rootfs construction](../../packages/guest-image/rootfs/build.sh) also installs
standalone tools such as `uv`. Avoid copying package versions into this design:
the built image determines the actual versions.

Home-based version managers and their runtimes are installed when needed. The
base does not preinstall nvm, rustup, Bun, Go, a JDK, or Docker. On first boot,
initialization copies `/etc/skel` to the empty home and sets ownership. The shell
skeleton keeps installer-appended environment setup outside the interactive-only
guard, so subsequent [login shell jobs](runner.md#shell-jobs) can find user tools.

A runner update must reach the rootfs actually pinned by a machine. Rebuilding the
configured rootfs and restarting the manager does not upgrade an existing base
reference. Acceptance checks the guest PID 1 executable hash after boot and again
after hibernate/wake, then exercises shell and native commands.

### Save a generation

The provisioner serializes disk operations per device. Checkpoint captures both
writable disks at one VM pause point and publishes the generation only after both
images are durable. Hibernate stops the VM and saves its writable disks; no memory
snapshot is required.

| Failure point | Required recovery |
| --- | --- |
| Before generation publication | Keep the prior committed generation and newer working files for retry. |
| After generation publication | Use the new committed generation. |
| Working disks remain unsaved | Do not boot a stale stored generation over them. |
| Manager or backend restarts | Stop previous VM writers before saving or starting another VM. |

The runner can sync filesystems, but saving must also work when the guest is
broken. Journal replay provides filesystem crash consistency, not application
transactions across files or volumes. Skipping a checkpoint requires evidence
that neither writable disk changed. Two VMs must never write the same disk.

`DirMachineImageStore` publishes immutable generations and atomically advances
`current.json` after syncing files and directories. It retains the current and
previous generation. Cross-worker storage and execution fencing remains a
[scaled-deployment requirement](sessions-and-targets.md#implementation-ownership-and-checks).

## Lifecycle and capacity

The logical device id survives wake, shutdown, crashes, and reset. Concurrent
first-use requests join the same device allocation and boot. Metadata operations
can reference Cloud before any VM exists.

| Transition | Result |
| --- | --- |
| Off → booting → running | Boot the pinned disks, rotate the device token, and wait for the runner. |
| Running → checkpoint → running | Publish a consistent disk generation and resume execution. |
| Running → saving → off | Save writable disks and release the VM. |
| Resetting → running | Replace the system layer, retain home, and boot the reset generation. |
| Failed boot or save | Expose the failed operation and allow explicit recovery without creating another device. |

Idle reclamation requires the configured idle window, no admitted device demand,
no relevant active agent trees, and no running jobs. Evaluate activity across all
conversations using this device. Browser commands count through their Host
operation leases; idle browser grants, attachments, and
backend metadata observers do not. Cleanup and checkpoints are maintenance:
they serialize against conflicting transitions without restarting the idle clock.

The [shared coordinator](resource-lifecycle.md#retiring-dependencies-together)
retires dependent browser resources before the adapter saves disks and stops the
VM. If both deadlines are due, one coordinated retirement handles them. This
does not alter the browser's conversation ownership or the device's user ownership.
Only new demand may wake a stopped Cloud; a cleanup callback cannot do so.

The hard lifetime cap measures the running device lifetime and permits stopping
unattended jobs when no relevant agent turn is active. It is separate from idle
eligibility. Reserve admission and recheck active turns and demand other than the
cap-eligible jobs. A viewer or admitted interactive operation postpones the cap
transition. Cancel eligible unattended jobs and await their released leases
before proceeding with browser retirement and machine shutdown. The cap does
not authorize dropping a live browser viewer.
Crash-loop protection stops repeated automatic boots while leaving reset available.
Checkpoint timing remains Cloud policy and uses shared maintenance admission;
a checkpoint preserves the browser's live process and is not a browser retirement.

The shared coordinator owns idle intervals and scheduling for each machine
independently. Device and agent-tree activity notifications reset demand intervals;
periodic eligibility checks cover durable conversation bindings. Checkpoint and
hard-cap schedules remain Cloud policy and use the same coordinator clock. VM
state, reset journals and disk recovery remain in the managed adapter.

Guest resource limits are per user; the backend also caps total active machines.
Current configuration defaults are:

| Owner | Setting | Default |
| --- | --- | --- |
| Machine manager | vCPU / guest RAM | 2 / 2 GiB |
| Machine manager | Initial system / home capacity | 1 GiB / 1 GiB |
| Backend lifecycle | System / home quota | 16 GiB / 32 GiB |
| Backend lifecycle | Idle window / unattended-job cap | 10 minutes / 24 hours |
| Backend lifecycle | Checkpoint interval | 15 minutes |
| Backend lifecycle | Concurrent running or booting machines | 16 |

Initial disk capacity is not quota; growth applies separately to each volume.
Archiving conversations and deleting project metadata do not delete Cloud.
Account-data destruction requires a separate explicit retention/deletion policy.

Image copying requests filesystem cloning where supported. The design does not
guarantee sparse or constant-cost copies. Host capacity must account for working
disks and retained generations; actual allocation depends on the filesystem.
Measure disk allocation, command-ready latency, and host memory on the deployment
rather than inferring them from guest RAM or nominal disk sizes.

## System reset

The owner can request reset through the backend even when the guest cannot boot.
The backend records an operation id, target base version, phase, and error so a
retry refers to the same operation. The user-facing consequence is explicit:
Cloud tasks stop, system packages and configuration are replaced, and home stays.

The reset sequence is:

1. Close device admission and interrupt affected turns and jobs across conversations.
2. Stop the guest and preserve its latest home image.
3. Create an empty system image for the selected base.
4. Atomically publish that system image with the preserved home.
5. Announce the reset to affected conversation contexts, boot, and report readiness.

Pending work receives interruption instead of automatic replay. A failed home
save must not publish a replacement generation or discard the working home.
Before publication, recovery retains the old generation; after publication,
recovery uses the new one and can retry boot. Admission stays closed until the
committed state is established. Reset does not run repair commands inside the guest.

Device identity, project metadata, and conversation history stay unchanged.
Each agent node observes the reset through its next persisted execution context.
Lifecycle status and operation phase remain distinct: `resetting` can contain
stopping, saving, rebuilding, or booting. Failure retains the operation and error.

## Isolation and joining

The manager supports direct Firecracker launch and jailer launch. Direct mode
runs with KVM access and Firecracker's seccomp filter. Jailer mode adds the
jailer's chroot, namespaces, cgroup, and slot user. The privileged launcher is a
trusted infrastructure adapter, not a boundary against the manager account.

The networking setup prepares taps, forwarding, and firewall policy. Guests can
reach the configured backend endpoint and public destinations. Other private and
link-local destinations are blocked, inbound services are not exposed, and no
host directories are mounted into a Firecracker guest.

The backend mints a managed-device token for boot and stores its hash. Kernel
arguments supply the guest's backend URL, token, address, gateway, and DNS.
The runner keeps the token off persistent guest disks and connects directly.
A managed runner without a token is rejected instead of entering device pairing.
Each token permits one live connection; the backend checks user ownership on
control operations.

## Verification

Scripted-provider and fake-provisioner tests cover first-use races, shared-device
admission, user isolation, crash loops, and reset failures on either side of
publication. Linux/KVM smoke tests cover direct and jailer launch, persistence,
reset with retained home, broken-guest reset, volume growth, and interrupted saves.
KVM execution results and capacity measurements belong to acceptance reports.
