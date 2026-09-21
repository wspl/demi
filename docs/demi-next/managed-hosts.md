# Managed Cloud hosts

After a user installs a system package and writes a project under `/home/demi`,
stopping and waking Cloud preserves both. Reset replaces the system and keeps
home. All of that user's conversations share the same files, packages, ports,
and resource budget. A workspace is a starting directory, not an isolation
boundary.

Cloud is one persistent logical device per user, with at most one active gVisor
sandbox running its runner. **Every deployment provides Cloud.** Conversations
without a paired device and process-backed provider work need it. The sole
execution platform is **gVisor with systrap**. There is no selectable hypervisor,
ordinary-container fallback, or unsandboxed execution on the backend host.

This document owns provisioning, isolation, persistence, and reset. The
[execution-target contract](sessions-and-targets.md) owns Host access and
admission; [resource lifecycle](resource-lifecycle.md) owns the idle rule and its
clock. [Cloud setup](../managed-hosts-setup.md) owns deployment configuration;
[Cloud images](../cloud-images.md) owns image production. Package responsibilities
are authoritative in [package boundaries](../package-boundaries.md).

## Provisioning

The backend owns device identity, authentication, Cloud policy, and durable
allocation/reset intent. Its lifecycle coordinator admits work and schedules
maintenance. The machine manager owns sandbox processes, mounts, networking,
resource enforcement, and stored generations. It has no users, conversations,
transcripts, or control database.

```text
Backend (Linux or the developer's Mac)
    | provisioner requests over a restricted Unix socket
    v
Machine manager (Linux)
    | OCI bundle and runsc lifecycle commands
    v
One user's gVisor/systrap sandbox
    init -> runner -> jobs and native services -> Chrome
    |
    +---- authenticated outbound connection ----> Backend
```

The manager invokes the pinned `runsc` distribution directly using its OCI
runtime interface. Docker and containerd are not runtime dependencies. An OCI
bundle describes a prepared root directory, process, mounts, namespaces, and
resource limits; it is not another image build on each wake. `runsc` supplies the
Sentry, which implements the sandbox's Linux interface, and its filesystem
helper, the Gofer. The manager supervises their lifetime, including failed starts.
See the upstream [OCI interface](https://gvisor.dev/docs/user_guide/quick_start/oci/).

A Linux execution host can be a VPS without KVM. It must still support the Linux
facilities listed in setup. On a Mac, the manager and sandboxes run inside one
Lima Linux VM with the Mac's native CPU architecture. systrap needs no nested
virtualization. Backend storage and the web application can stay on the Mac.
gVisor itself does not run on macOS. See the upstream
[platform guide](https://gvisor.dev/docs/architecture_guide/platforms/).

### Control and ownership

The manager serves newline-delimited JSON over a restricted Unix socket.
Requests contain `{ id, op, params }`; replies identify that request and contain
a result or an error. Death events identify the device whose sandbox exited.
`packages/machines/src/protocol.ts` owns the wire schemas. Socket access grants
infrastructure control: there is no manager TCP listener or application login.
The socket's dedicated owner/group includes only trusted backend infrastructure. The backend never runs as root merely to reach this socket.

The manager is a trusted, privileged Linux service because it prepares mounts,
loop devices, network namespaces, firewall rules, and cgroups. Its private mount
namespace contains the storage mounts. Its data directory, runtime bundles, and
control files are inaccessible to ordinary host users. No sandbox receives these
infrastructure privileges. Before mounting a working image, the manager pins its
private mount namespace with a root-only namespace handle under its runtime
directory, registered in the execution host's mount namespace. After a manager
crash, the handle's owner record requires recovery with the same state directory;
changing directories cannot abandon old writers. The replacement enters that saved namespace to thaw filesystems and fence
the old runtimes before publishing storage. Only after recovery does it release
the handle and register its own namespace. This keeps frozen mounts reachable
even when the original manager process has gone; service shutdown recovery uses
the same path.

One manager holds an exclusive lock on its state directory. Each device's
operations serialize under that manager. The working manifest identifies the
generation, and a separate runtime record identifies the unique boot and network
slot; resource paths follow from these identities. The presence of these records
determines which recovery steps are required. Neither contains a device token.
Runtime identity includes a unique boot id, so a
late death event cannot stop a replacement sandbox. Startup reconciles runtime
state and removes old writers before accepting work. Losing the client socket
does not establish that an operation failed: the backend queries/reconciles state
before retrying. If a Cloud marked running has no connected runner, Host admission
queries its runtime state. A stopped runtime starts again over saved storage;
an existing runtime gets the bounded runner reconnect interval without rotating
its credential or killing its work. Concurrent requests join that recovery.
Repeated reset requests use the same operation id.

`ManagedHostProvisioner` retains these operations:

| Operation | Meaning |
| --- | --- |
| `currentBaseVersion`, `imageState` | Read the selected base and a device's committed generation. |
| `runtimeState` | Read whether the manager owns an active runtime after serializing with its transitions. |
| `reconcile` | Recover incomplete operations and establish saved, stopped devices. |
| `wake` | Create first-use storage or recover existing storage, then start one sandbox with the supplied boot credential. |
| `checkpoint` | Publish paired system/home storage while preserving the running processes. |
| `hibernate` | Stop execution, save storage, and release runtime resources. No memory image is saved. |
| `growVolume` | Increase one writable filesystem's capacity within policy limits. |
| `reset` | Publish a clean system paired with retained home, idempotently by operation id; do not boot. |
| `close`, `onDeath` | Drain/save on shutdown and report unexpected runtime loss. |

A successful `wake` means the runtime started. Backend readiness additionally
requires the authenticated runner and command transport; starting `runsc` alone
must not mark Cloud ready. Closing the backend saves its machines and disconnects
without terminating the manager service.

## Images

A committed generation contains `generation`, `baseVersion`, `resetId`,
`systemBytes`, and `homeBytes`. The byte fields are filesystem capacities, not
used bytes, host allocation, or quota. The generation identifies two immutable
ext4 image files. Only private working copies are mounted writable.

| Storage | Contents | Stop and wake | System reset |
| --- | --- | --- | --- |
| Pinned read-only base directory | OS, shipped tools, runner, and init | Keep the exact base | Select the reset base |
| System image | Host OverlayFS upper/work directories; changes outside the separate mounts | Preserve | Replace with an empty layer |
| Home image | `/home`, including projects and user tools | Preserve | Preserve |
| Runtime mounts | `/run`, `/tmp`, `/dev/shm`, boot credential, resolver configuration | Recreate | Recreate |

The manager extracts a verified base once, before making it available. At wake,
it mounts the two working ext4 images through loop devices in its private mount
namespace. Linux OverlayFS combines the immutable base with upper/work on the
system filesystem; the resulting directory is the OCI root. The home filesystem
is a separate OCI bind mount at `/home`. The images and loop devices themselves
are never visible inside the sandbox. All user-created files, including overlay
whiteouts and metadata, belong to the writable images, not the shared base.

This retains bounded filesystem capacities and paired generations without
requiring a guest kernel. The host storage filesystem can be XFS with reflinks,
btrfs, or ext4; cloning is an optimization, not a correctness requirement. Copies preserve
sparse ranges so unused capacity does not become allocated host storage. The
writable ext4 filesystems remain distinct from that host storage filesystem.
Overlay layout and supported mount options are fixed by the image/storage
format; they are not per-user configuration. See the Linux
[OverlayFS contract](https://docs.kernel.org/filesystems/overlayfs.html).

The runtime uses `--overlay2=none`: its default temporary overlay would hide
system writes from this storage. Root and external file access both use shared
mode (`--file-access=shared --file-access-mounts=shared`), with no forced private
page cache. The manager must be able to save the host-backed files after pausing
sandbox tasks. This is a deliberate persistence choice whose performance must
be measured. It differs from the evaluation's default root access mode. See the
[gVisor filesystem guide](https://gvisor.dev/docs/user_guide/filesystem/) and the
pinned runtime's [file I/O implementation](https://github.com/google/gvisor/blob/release-20260914.0/pkg/sentry/fsimpl/gofer/regular_file.go).

Deploying a base makes it available for new devices and resets; ordinary wake
never silently upgrades an existing base. A base identifies its architecture and
complete build manifest. CPU architectures are not interchangeable: moving
persisted system/home state between architectures is outside this contract.
Runner output, command contexts, caches, and browser processes under `/run/demi`
are temporary. Durable results must be written outside runtime mounts.

### Container initialization

The OCI process is a shipped minimal init (`tini`) running as `demi`, UID/GID
1000, which starts the runner and reaps orphaned descendants. The manager supplies
mounts, network configuration, and resource limits before start. The runner is
an ordinary process; it does not mount a root filesystem, configure host
networking, parse kernel boot arguments, or infer a boot mode from PID 1.

The manager initializes a newly allocated home from the base's `/etc/skel` and
records that initialization with storage publication. It never repeats this
based merely on an empty directory, and never recursively changes an existing
home's ownership at wake or reset. Jobs and Host file operations use `demi`.
Passwordless sudo permits administration inside the sandbox. Home configuration,
including a broken shell profile, survives reset.

Init forwards termination and reaps children. A runner exit ends the sandbox;
init does not hide repeated runner crashes by restarting it. The manager applies
a bounded graceful-stop interval, then terminates the remaining runtime and
waits for all Sentry/Gofer processes before releasing storage. Orphaned Chrome,
crashpad, native services, and sandbox helper processes cannot survive stop.

### Save a generation

A checkpoint is a disk operation, not gVisor memory checkpoint/restore. The
manager reserves device maintenance and performs the following sequence:

1. Pause sandbox tasks through `runsc` and confirm the instance is still alive.
2. Flush and freeze both host-mounted writable ext4 filesystems. The frozen
   mounts are inside the working images, not the manager's parent state filesystem.
3. Clone or copy both backing files into a new staging generation while both
   filesystems remain frozen. Sync the staged files and manifest.
4. Thaw both filesystems and resume the same runtime. Always attempt both thaw
   operations and resume on failure or cancellation; do not leave a paused
   sandbox or frozen filesystem behind.
5. Publish the staged generation by atomic rename, sync its directory, and
   atomically replace and sync `current.json` and its parent directory.

The pair is captured during one interval with no sandbox writer. Pausing alone
is not a persistence barrier: shared host-backed writes, host filesystem
flush/freeze, and durable publication are all required. Acceptance must exercise
ordinary writes, writable mappings, rename/unlink, and cross-volume writers.
An unresponsive runtime cannot produce a successful live checkpoint. Report the
failure; do not claim a checkpoint by silently restarting the user's browser.
If thaw/resume fails, close admission and recover the device as a runtime loss.

Hibernate instead stops all writers, releases Gofer references, unmounts the
merged root and writable filesystems, and publishes their saved images. This
path must work without a healthy runner or user shell. A forced stop promises
filesystem crash consistency; unsynced application buffers can be lost. Neither
path promises an application transaction across files or volumes.

| Failure boundary | Recovery |
| --- | --- |
| Before publication | Keep the prior committed pair and the newer working pair for recovery. |
| After publication | Use the new committed pair. Never combine volumes from different generations. |
| Working state contains newer writes | Recover it before wake; do not overwrite it from an older generation. |
| Manager, backend, or host restart | Fence old writers, thaw surviving mounts where necessary, recover filesystems, then save or start. |
| Save fails or storage is full | Keep working data and the error; release execution resources where safe and block a stale boot. |

The immutable store retains current and previous generations and removes older
committed pairs after durable publication. Staging data is removable only after
reconciliation establishes that no pending operation needs it; failed working
data is never garbage. Base directories are retained automatically. An operator
may remove an unreferenced base only with backend and manager stopped, after
checking the configured image, committed generations, working records, and backend
reset intents. This keeps a base selected by a durable reset available even before
the backend has dispatched that reset to the manager.
Snapshot sharing does not make retained generations free; capacity monitoring
includes bases, staging, working files, and retained generations. Cross-host
storage and execution fencing remain a
[scaled-deployment requirement](sessions-and-targets.md#implementation-ownership-and-checks).

## Lifecycle and capacity

Device identity survives wake, stop, runtime loss, and reset. Concurrent first
uses join one allocation and wake. Metadata alone never creates a sandbox.

| Transition | Result |
| --- | --- |
| Off → booting → running | Recover the pinned storage, rotate the token, start a sandbox, and await the runner. |
| Running → checkpoint → running | Save a paired generation and resume the same processes. |
| Running → saving → off | End processes, save storage, and release the runtime. |
| Resetting → running | Publish a fresh system with retained home, then boot. |
| Failed boot, save, or reset | Keep the device and operation error; retry only after recovery establishes the authoritative state. |

The [idle rule](resource-lifecycle.md) applies across every conversation using
this device. Tabs, resident services, attachments, and passive observers do not
keep it active. Maintenance does not restart the idle clock. Stopping Cloud ends
all in-sandbox browser/native state and its [exposes](expose.md#lifetime); cleanup
must not wake it or send per-conversation release to a stopped device.

The hard lifetime cap can stop unattended jobs only after reserving admission
and rechecking active turns and other demand. An admitted interactive operation
postpones it. Cancel eligible jobs and await released leases before stopping.
Crash-loop protection stops repeated automatic boots while retaining reset.
Scheduling belongs to the shared lifecycle coordinator; Cloud policy supplies
checkpoint and lifetime schedules.

| Owner | Setting | Default |
| --- | --- | --- |
| Machine manager | CPU budget / memory limit per sandbox | 2 CPUs / 2 GiB |
| Machine manager | Writable system / home initial capacity | 1 GiB / 1 GiB |
| Machine manager | `/dev/shm` / `/tmp` / `/run` size limits | 256 MiB / 256 MiB / 256 MiB |
| Backend policy | System / home maximum capacity | 16 GiB / 32 GiB |
| Backend lifecycle | Idle window / unattended-job cap | 1 hour / 24 hours |
| Backend lifecycle | Checkpoint interval | 15 minutes |
| Backend admission | Concurrent running or booting devices | 16 |

CPU is a cgroup scheduling budget, not a virtual CPU allocation. cgroup v2 limits
cover the complete sandbox, including Sentry and Gofer; temporary mounts count
toward memory and do not create extra allowances. Disable sandbox swap. Set host
`pids.max` to 1024 and the sandbox process soft/hard `RLIMIT_NPROC` to 1024. These count different things: gVisor does not
map each sandbox process to a host PID. The latter limits ordinary-user jobs,
not sandbox root; CPU and memory remain the resource boundary for sudo workloads.
Reserve capacity for the host and manager rather than treating 16 devices as safe
on every VPS.

Growth changes only one working filesystem: enlarge its sparse backing file,
refresh the loop device capacity, and grow ext4 on the host. It is serialized
against saving and reset, monotonic, bounded by backend policy, and recoverable
if interrupted between those steps. Record the actual filesystem capacity, not
merely the requested file size, before reporting success. A quota increase does
not promise physical disk space; admission and monitoring must handle host
`ENOSPC`. A sandbox cannot grow its own backing device.

Archiving conversations or deleting project metadata never deletes Cloud data.
Account-data destruction requires its own explicit retention/deletion policy.

## System reset

The owner can reset through the backend even when the runner, shell, or system
files are broken. The backend persists an operation id, selected base, phase,
and error. A retry resumes that operation, rather than selecting a newer base.

1. Reserve device admission and hold/interrupt affected work according to
   [shared Cloud coordination](sessions-and-targets.md#coordinate-shared-cloud-activity).
2. Stop the sandbox and preserve its latest home, including recoverable working
   changes newer than the last checkpoint.
3. Create an empty system layer for the selected base.
4. Durably publish the new system and preserved home as one generation carrying
   the reset operation id.
5. Announce reset to affected execution contexts, rotate the token, and boot.
   Report success only when the runner is ready.

A failed home save blocks publication and retains the working home. Before
publication recovery retains the old pair; after publication it uses the new
pair and may retry boot. Admission remains held until an authoritative state
is established; a terminal failure is surfaced to waiting operations. Reset
never runs repair commands inside the broken sandbox. Interrupted side effects
are not automatically replayed.

Device identity, projects, and history remain unchanged. Lifecycle status and
reset phase are distinct: `resetting` may include stopping, saving, rebuilding,
or booting. Each agent node observes reset in its next persisted context.

## Isolation and joining

The isolation boundary is the gVisor sandbox, not a workspace, Docker namespace,
or hardware VM. Each user gets a separate Sentry, filesystem view, network
namespace, and resource group. Infrastructure still trusts the Linux kernel,
manager, runsc distribution, and image builder. The product supports ordinary
Linux development and browsers, not arbitrary kernel features or privileged
nested Docker. Unsupported operations return errors; they never widen isolation.

The runtime inputs are pinned in
[the runtime release manifest](../../packages/machines/runtime/release.json).
amd64 uses the verified upstream distribution. arm64 uses the same source with
the shipped `SECCOMP_RET_TRAP` register fix: Linux preserves the first argument
in X0 when delivering SIGSYS, while the upstream implementation overwrites it
with the syscall number. Chrome's sandbox signal handler depends on that argument
and crashes without the fix. The ARM build has its own version and a native-versus-
sandbox ABI regression probe. It retains both gVisor isolation and Chrome's
sandbox. See the upstream
[trap handling](https://github.com/google/gvisor/blob/release-20260914.0/pkg/sentry/kernel/seccomp.go)
and [Chrome's signal handler](https://github.com/chromium/chromium/blob/main/sandbox/linux/seccomp-bpf-helpers/sigsys_handlers.cc).
Runtime upgrades require the same acceptance matrix, but do not change user
storage format.

The runtime profile fixes `--platform=systrap`, `--network=sandbox`, and
`--allow-suid=true`. OCI `noNewPrivileges` is false so sudo can work. The
OCI bounding set is `CHOWN`, `DAC_OVERRIDE`, `FOWNER`, `FSETID`, `KILL`, `SETGID`,
`SETUID`, `SETPCAP`, `NET_BIND_SERVICE`, `SYS_CHROOT`, and `SETFCAP` (all with the
`CAP_` prefix). The initial UID 1000 process has empty effective, permitted,
inheritable, and ambient sets; setuid sudo may acquire privileges only within
that bounding set. The profile excludes host administration, module loading,
raw devices, and network administration. Test sudo and Chrome with this exact set.
Chrome's own sandbox remains enabled. No privileged mode, host networking,
unconfined runtime seccomp, host PID namespace, or runtime socket is exposed.
Keep runsc's standard Directfs security profile; changes require a design and
acceptance update, not a per-user escape hatch.

Only the prepared root, this device's home, and explicit runtime mounts reach
Gofer. No checkout, Mac home, Docker socket, manager state, arbitrary host path,
or other user's mount is available. `/proc` and devices are sandbox views;
host sysfs and writable cgroups are not mounted. The workload cannot supply
OCI annotations, host paths, runtime flags, or network/cgroup identities.

### Networking

The manager creates a dedicated Linux network namespace and veth connection per
sandbox. runsc builds its isolated network stack from that namespace. Host
firewall rules are installed before start and match the device interface and
source address; the workload cannot replace them. There are no guest taps or
per-user host port publications.

Allow outbound public traffic, replies to established connections, the exact
configured backend address/port, and the configured DNS resolver on TCP/UDP 53.
Deny other private, loopback, link-local, metadata, multicast, reserved, and
other-tenant destinations, including the host's own services. Enforce policy on
both forwarded traffic and traffic addressed to the execution host. Disable IPv6
in the initial profile so it cannot bypass IPv4 policy. Resolver configuration
uses an address reachable from the sandbox, never a container engine's loopback
resolver. Filtering is by destination address, so DNS rebinding does not grant
private-network access. Exceptions for a private backend or resolver are exact
endpoints, not whole subnets; address changes require revalidation and rule update.

A service's public URL uses the backend's [Host expose](expose.md) relay over the
runner connection. The same connection carries files, commands, and browser
streams through normal Host access. No infrastructure transport is an alternate route for conversation operations.

### Managed boot credential

The backend mints a managed-device token per boot and stores its hash. The
manager receives `{ backendUrl, deviceToken }` only for that boot, validates it
against the runner-protocol schema and configured backend allowlist, and supplies
a private read-only credential file on a temporary mount. The runner accepts
`--managed-boot <path>`, reads and validates the record before connecting, and keeps its installation state under `/run/demi`.
The token never appears in process arguments, image layers, persistent working
files, OCI environment, logs, or stored generations. Only the non-secret file
path is part of the OCI process arguments. Runtime metadata containing sensitive
paths stays in an owner-restricted temporary directory; cleanup removes it on
both failed start and stop.

A managed runner with missing or invalid boot credentials fails; it never enters
pairing. Each token permits one live connection and is scoped to that device.
Rotation rejects the previous boot's token. Sandbox root can inspect its own
runner and credential; the boundary is against other users and infrastructure,
not against the owner of that Cloud. The ordinary paired-device registration and
persistent token path remain separate, as defined by the [runner](runner.md).

## Verification

Scripted-provider and fake-provisioner scenarios cover allocation races,
admission, identity, token rotation, crash loops, idle policy, and reset failures.
Real provisioner acceptance covers the exact shipped runtime, image, storage,
and network profile, on Linux amd64 and arm64 without KVM, and on arm64 Lima.
Compilation or a Docker evaluation container cannot substitute for these checks.

Required real checks include system package installation across wake, retained
home across reset, broken-runner reset, live checkpoint with a running browser,
file and mmap durability, growth, disk-full failures, restart at each publication
boundary, runtime/OOM loss, and simultaneous users. Confirm private/metadata and
cross-user traffic refusal, the permitted backend/DNS paths, and no residual
processes, mounts, loop devices, network rules, credentials, or listeners after
success, failure, and cancellation. Inject faults after acquiring each resource.

Exercise shell, native commands, provider CLI installation, and the real browser
panel through normal Host access on both Cloud and a paired device. Automated
checks use scripted models. Measure image preparation, storage setup, runsc
start, runner authentication, first command, first Chrome open, subsequent open,
first live frame, checkpoint pause, and stop separately; include cold/warm runs,
CPU architecture, memory peaks, and concurrent load. Do not turn a single observed
runtime-start duration into an end-to-end startup promise.

## Implementation status

The direct OCI manager, persistent volume store, ordinary runner boot, root
archive pipeline, and Linux/Lima installers implement this contract. The old
hypervisor launcher, guest kernel pipeline, and runner PID 1 boot path are
removed. A deployment uses a new state directory and does not import old state.

The opt-in `real-gvisor.e2e.test.ts` runs a real manager and packaged image with a
scripted model. It covers file ownership, shared device identity, browser frames
and input, online growth, paired checkpoint, stop/wake, broken-system reset, and
login-shell tool installation. Run it with `DEMI_GVISOR_E2E=1`, the manager's
`DEMI_GVISOR_E2E_SOCKET`, an allowed `DEMI_GVISOR_E2E_PUBLIC` backend URL, and a
local copy of the image manifest in `DEMI_GVISOR_E2E_MANIFEST`.

The [evaluation report](../gvisor-evaluation.md) records measured environments and
remaining acceptance limits separately from this design. Passing the functional
suite does not establish production capacity, concurrent-load latency, or
multi-worker failover.
