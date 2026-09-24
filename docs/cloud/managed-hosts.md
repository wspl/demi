# Managed Cloud hosts

After a user installs a system package and writes a project under `/home/demi`,
stopping and waking Cloud preserves both. Reset replaces the system and keeps
home. All of that user's conversations share the same files, packages, ports,
and resource budget. A workspace is a starting directory, not an isolation
boundary.

Cloud is one persistent logical device per user, with at most one active gVisor
sandbox running its runner. **Every deployment provides Cloud.** Conversations
without a paired device and process-backed provider work need it. The sole
execution platform is **gVisor with systrap**. There is no selectable
hypervisor, ordinary-container fallback, or unsandboxed execution on the backend
host.

This document owns provisioning, isolation, persistence, reset, and the stored
disk generations of Cloud machines. The
[execution-target contract](../execution/sessions-and-targets.md) owns Host
access and admission; [resource lifecycle](../execution/resource-lifecycle.md)
owns the idle rule and its clock. [Cloud setup](setup.md) owns deployment
configuration; [Cloud images](images.md) owns image production.
[Crates and packages](../architecture/crates-and-packages.md) defines the
manager's crates and their dependencies.

## Provisioning

The backend owns device identity, authentication, Cloud policy, and durable
allocation and reset intent. Each user's Cloud machine belongs to that user's
[shard](../architecture/concurrency.md#the-user-shard), which admits work to
it, runs its idle watch and maintenance, and drives its wake, hibernation,
checkpoints, and reset. The one Cloud fact that spans users, how many machines
are not stopped, is a capacity count at the backend's edge
([Lifecycle and capacity](#lifecycle-and-capacity)). The machine manager owns
sandbox processes, mounts, networking, resource enforcement, and stored
generations. It has no users, conversations, transcripts, or control database.

```text
Backend (Linux or the developer's Mac)
    | requests over a restricted Unix socket
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
resource limits; it is not another image build on each wake. `runsc` supplies
the Sentry, which implements the sandbox's Linux interface, and its filesystem
helper, the Gofer. The manager supervises their lifetime, including failed
starts. See the upstream
[OCI interface](https://gvisor.dev/docs/user_guide/quick_start/oci/).

A Linux execution host can be a VPS without KVM. It must still support the Linux
facilities listed in [setup](setup.md#linux-requirements). On a Mac, the manager
and sandboxes run inside one Lima Linux VM with the Mac's native CPU
architecture. systrap needs no nested virtualization. Backend storage and the
web application can stay on the Mac. gVisor itself does not run on macOS. See
the upstream [platform guide](https://gvisor.dev/docs/architecture_guide/platforms/).

### Linux control

The manager is one Linux executable. It controls the host through the kernel's
own interfaces rather than administration tools, so it handles results and
errors as values instead of parsing command output:

- mounts, loop devices, filesystem freezes, namespaces, and locks through system
  calls and ioctls (`rustix`);
- network interfaces, addresses, and routes through netlink (`rtnetlink`);
- cgroups and network settings through files under `/sys` and `/proc`;
- each boot's OCI bundle as typed OCI specification structures (`oci-spec`).

It runs a program only where no library does the job:

| Program | Why the manager runs it |
| --- | --- |
| `runsc` | The sandbox runtime; the manager drives the pinned distribution through its OCI commands. |
| `mke2fs`, `e2fsck`, `resize2fs` | Create, check, and grow the ext4 images; no Rust library creates or repairs ext4 filesystems. `resize2fs` both grows a mounted filesystem and completes an interrupted growth offline. |
| `bsdtar` | Extract a base archive with numeric ownership, modes including setuid bits, ACLs, and extended attributes intact, refusing paths that leave the target directory. |
| `nft` | Apply the firewall table as one transaction ([Networking](#networking)). |

Filesystem checks and resizes run without a deadline: a repair lasts as long as
the damage it finds, and killing one midway does more harm than waiting.
`runsc` commands have deadlines. Copying an image and reading a
filesystem's capacity happen inside the manager ([Images](#images)).

### Control and ownership

The manager serves newline-delimited JSON over a restricted Unix socket. For
example, the backend hibernates a Cloud, and later the manager reports that
another device's sandbox exited:

```text
backend -> manager   {"id":"7","op":"hibernate","params":{"deviceId":"<device>"}}
manager -> backend   {"type":"ok","id":"7","result":null}
manager -> backend   {"type":"death","deviceId":"<other device>"}
```

Each request carries an id the client chooses, an operation, and its
parameters. The reply names the request and carries a result or an error
message. A `death` event goes to every connection when a device's sandbox exits
without being asked to stop. The `machines-protocol` crate defines every
message, and both the manager and the backend link it.

- A line holds one message of at most 1 MiB; the largest real message is a few
  kilobytes.
- A line that is longer, is not valid UTF-8 JSON, names an unknown operation, or
  carries malformed parameters drops its connection. The manager keeps serving
  its other connections; lines behind the bad one are not served.
- An invalid device id or base version is an ordinary error reply, and the
  connection stays usable.
- Requests on one connection run concurrently. Replies arrive in completion
  order and match their requests by id.
- A reply for a connection that has closed is discarded, and its operation
  still completes.

Socket access grants infrastructure control: there is no manager TCP listener
or application login. The socket's dedicated owner/group includes only trusted
backend infrastructure. The backend never runs as root merely to reach this
socket.

| Operation | Meaning |
| --- | --- |
| `current_base_version`, `image_state` | Read the configured base and a device's committed generation. |
| `runtime_state` | Read whether the manager runs a sandbox for the device, after the device's earlier operations. |
| `reconcile` | Stop and save every device, recover incomplete operations, and install the network policy again. |
| `wake` | Create first-use storage or recover existing storage, then start one sandbox with the supplied boot credential. |
| `checkpoint` | Publish paired system/home storage while preserving the running processes. |
| `hibernate` | Stop execution, save storage, and release runtime resources. No memory image is saved. |
| `grow_volume` | Increase one writable filesystem's capacity within policy limits. |
| `reset` | Publish a clean system paired with retained home, idempotently by operation id; do not boot. |

The backend sends `reconcile` when it starts, before it serves requests, and
again over a live connection when it closes. It keeps one connection to the
manager, opened on first use. When that connection drops, every call in flight
fails with a manager-unavailable error, and the next call connects again. The
backend routes each death event to the shard of the device's owner
([Runtime model](../backend/backend.md#runtime-model)).

A successful `wake` means the runtime started. Backend readiness additionally
requires the authenticated runner and command transport; starting `runsc` alone
must not mark Cloud ready. Closing the backend ends each Cloud's open file
transfers and user streams, then saves the machine and disconnects, without
terminating the manager service
([Startup and shutdown](../backend/backend.md#startup-and-shutdown)).

The manager is a trusted, privileged Linux service because it prepares mounts,
loop devices, network namespaces, firewall rules, and cgroups. Its private mount
namespace contains the storage mounts. Its data directory, runtime bundles, and
control files are inaccessible to ordinary host users. No sandbox receives these
infrastructure privileges.

One manager holds exclusive locks on its state directory and its runtime
directory. Each device has one worker that runs the device's operations one at
a time, in arrival order
([Machine manager](../architecture/concurrency.md#machine-manager) describes
the manager's threads). The manager learns that a sandbox exited by waiting on
it through `runsc`, and the device's worker takes the exit between two
operations, never during one. `reconcile`
and shutdown wait for the device operations in flight, and every request that
arrives after them waits until they finish.

The working manifest identifies the generation, and a separate runtime record
identifies the unique boot and network slot; resource paths follow from these
identities. The presence of these records determines which recovery steps are
required. Neither contains a device token. Runtime identity includes a unique
boot id: the worker ignores the exit of an earlier boot, so a late death event
cannot stop a newer sandbox. After the current boot exits, the worker stops what
remains, saves the working pair, and sends the death event, even when stopping
or saving fails.

Losing the client socket does not establish that an operation failed: the
backend queries or reconciles state before retrying. If a Cloud marked running
has no connected runner, Host admission queries its runtime state. A stopped
runtime starts again over saved storage; an existing runtime gets 60 seconds
for its runner to reconnect, without rotating its credential or killing its
work. Concurrent requests join that recovery. Repeated reset requests use the
same operation id.

### Startup and recovery

Before it serves a request, the manager:

1. Validates its configuration ([Cloud setup](setup.md#configuration)), and
   checks that it runs as root in a private mount namespace, that its programs
   resolve, and that `runsc` reports exactly the pinned version.
2. Takes its two locks. A second manager on the same state or runtime directory
   is refused, and the error names the path.
3. Recovers a namespace saved by an earlier manager (below) and removes
   leftover storage probes.
4. Enables the cgroup controllers it needs.
5. Stops and saves every device left behind: it removes staging directories,
   fences each recorded sandbox, and publishes each working pair.
6. Installs the network policy and imports the configured base
   ([Cloud images](images.md#import-and-publication)).
7. Pins its own mount namespace, checks with a small probe image that storage
   can be mounted, frozen, and copied, and opens its socket.
8. Reports readiness to systemd
   ([Cloud setup](setup.md#storage-and-service-setup)).

A manager's storage mounts exist only inside its private mount namespace. So
that they stay reachable after a crash, the manager pins that namespace before
mounting any working image: a root-only namespace handle under its runtime
directory, registered in the execution host's mount namespace, with an owner
record that names the state directory. If the manager dies, filesystems it
froze or mounted stay reachable through the handle.

The next manager recovers them before it publishes storage. The owner record
must name its own state directory; otherwise startup stops and asks for the
earlier manager to be recovered with its original directory, so changing
directories cannot abandon that manager's writers. Recovery runs as a separate
process of the manager's own executable, started inside the saved namespace
([Machine manager](../architecture/concurrency.md#machine-manager) explains
why it is a process). The recovery process inherits the manager's lock on the
state directory and refuses to run without it, so nobody can start it by hand
beside a live manager. It thaws surviving filesystems, fences the earlier
manager's sandboxes, and saves every working pair. Only after it succeeds does
the manager release the handle and, later, pin its own namespace. If it fails,
the handle stays, and the next start retries. Recovery has no deadline, for the
same reason filesystem checks have none.

Stopping the service uses the same path. On SIGTERM the manager stops accepting
connections, stops and saves every device, and then fails the requests still
waiting, so none can start a sandbox after the drain. It releases its handle
only if the drain succeeded. If the drain failed, the handle stays, and the
service's stop-post command recovers from the saved namespace, limited to
fencing and saving: a stopped service neither resolves names nor installs
firewall rules.

## Images

A committed generation contains `generation`, `baseVersion`, `resetId`,
`systemBytes`, and `homeBytes`; the same record answers `image_state`. The byte
fields are filesystem capacities, not used bytes, host allocation, or quota. The
manager reads a capacity from the image's ext4 superblock (block count times
block size), a documented on-disk format, instead of parsing a tool's output.
The generation identifies two immutable ext4 image files. Only private
working copies are mounted writable. A record that fails to decode is an error;
the manager never repairs it.

| Storage | Contents | Stop and wake | System reset |
| --- | --- | --- | --- |
| Pinned read-only base directory | OS, shipped tools, runner, and init | Keep the exact base | Select the reset base |
| System image | Host OverlayFS upper/work directories; changes outside the separate mounts | Preserve | Replace with an empty layer |
| Home image | `/home`, including projects and user tools | Preserve | Preserve |
| Runtime mounts | `/run`, `/tmp`, `/dev/shm`, boot credential, resolver configuration | Recreate | Recreate |

The manager keeps bases, generations, and working pairs under its state
directory, `DEMI_MACHINES_DATA`, which must be one filesystem
([Save a generation](#save-a-generation) explains why). Nothing of a machine
lives in the backend's data directory.

```text
<state directory>/
+-- images/
|   +-- bases/<baseVersion>/           an imported base: manifest.json, rootfs/
|   +-- <deviceId>/
|       +-- current.json               names the committed generation
|       +-- generations/<generation>/  manifest.json, system.ext4, home.ext4
+-- working/<deviceId>/                the working pair: manifest.json, system.ext4,
                                       home.ext4, and sandbox.json while it runs
```

The manager extracts a verified base once, before making it available
([Cloud images](images.md#import-and-publication)). At wake, it copies both
images of the committed generation into the device's working directory and
mounts the two working ext4 images through loop devices in its private mount
namespace. It attaches each image to a loop device it allocates itself, with
auto-clear set, and keeps the device number for the sandbox's lifetime:
unmounting the filesystem, or a failed mount, detaches the loop device, and
growth refreshes the device the sandbox already holds. Linux OverlayFS combines
the immutable base with upper/work on the system filesystem; the resulting
directory is the OCI root. The home filesystem is a separate OCI bind mount at
`/home`. The images and loop devices themselves are never visible inside the
sandbox. All user-created files, including overlay whiteouts and metadata,
belong to the writable images, not the shared base.

This retains bounded filesystem capacities and paired generations without
requiring a guest kernel. The host storage filesystem can be XFS with reflinks,
btrfs, or ext4; cloning is an optimization, not a correctness requirement. The
manager copies an image in its own process: it clones the file where the
filesystem supports reflinks, and otherwise copies only the ranges that hold
data, skipping holes and all-zero blocks, so unused capacity does not become
allocated host storage. A general-purpose file copy would fill the holes, and
running `cp` would put a process inside the checkpoint's frozen window. The
writable ext4 filesystems remain distinct from that host storage filesystem.
Overlay layout and supported mount options are fixed by the image/storage
format; they are not per-user configuration. See the Linux
[OverlayFS contract](https://docs.kernel.org/filesystems/overlayfs.html).

The runtime uses `--overlay2=none`: its default temporary overlay would hide
system writes from this storage. Root and external file access both use shared
mode (`--file-access=shared --file-access-mounts=shared`), with no forced
private page cache. The manager must be able to save the host-backed files after
pausing sandbox tasks. This is a deliberate persistence choice whose performance
must be measured. See the
[gVisor filesystem guide](https://gvisor.dev/docs/user_guide/filesystem/) and
the runtime's
[file I/O implementation](https://github.com/google/gvisor/blob/master/pkg/sentry/fsimpl/gofer/regular_file.go).

The service runs with a restrictive umask, so the files it creates for itself
stay private to root. What the sandbox must read never depends on that umask:
the manager gives each directory and file it creates for the sandbox an explicit
mode. A new system layer's upper directory, which OverlayFS presents as the
sandbox's `/`, and the root of a new home filesystem are 0755; the resolver and
hosts files are 0444; the boot credential is 0400 and owned by UID 1000. The
manager sets a mode only when it creates the directory or file; it never
changes an existing root directory's mode, which the user may have set.

Deploying a base makes it available for new devices and resets; ordinary wake
never silently upgrades an existing base. A base identifies its architecture
and complete build manifest. CPU architectures are not interchangeable: moving
persisted system/home state between architectures is outside this contract.
Runner output, command contexts, caches, and browser processes under
`/run/demi` are temporary. Durable results must be written outside runtime
mounts.

### Container initialization

The OCI process is a shipped minimal init (`tini`) running as `demi`, UID/GID
1000, which starts the runner and reaps orphaned descendants. The manager
supplies mounts, network configuration, and resource limits before start. The
runner is an ordinary process; it does not mount a root filesystem, configure
host networking, parse kernel boot arguments, or infer a boot mode from PID 1.

The manager initializes a newly allocated home from the base's `/etc/skel` and
records that initialization with storage publication. It copies the skeleton
into the new home image with symbolic links kept as they are and ownership set
to UID/GID 1000. It never repeats this based merely on an empty directory, and
never recursively changes an existing home's ownership at wake or reset. Jobs
and Host file operations use `demi`. Passwordless sudo permits administration
inside the sandbox. Home configuration, including a broken shell profile,
survives reset.

Init forwards termination and reaps children. A runner exit ends the sandbox;
init does not hide repeated runner crashes by restarting it. To stop a sandbox,
the manager asks every sandbox process to terminate and waits up to three
seconds, then deletes the runtime, kills whatever remains in the sandbox's
cgroup, and waits for all Sentry and Gofer processes to exit before releasing
storage. Orphaned Chrome, crashpad, native services, and sandbox helper
processes cannot survive stop.

### Save a generation

A checkpoint is a disk operation, not gVisor memory checkpoint/restore. The
device's worker performs the following sequence:

1. Pause sandbox tasks through `runsc`. If pausing fails, nothing is frozen, and
   the checkpoint reports that failure after step 3.
2. Run the frozen window as one job on a blocking thread: flush and freeze both
   host-mounted writable ext4 filesystems, copy both backing files into a
   staging directory and sync the copies, then thaw both filesystems. The
   frozen mounts are inside the working images, not the manager's parent state
   filesystem.
3. Resume the same runtime through `runsc`, whether or not step 2 succeeded.
4. Publish the copies as a new generation, as described below.

Nothing inside the frozen window waits for another task or process, so nothing
can interrupt it. A guard records each freeze as it is issued and thaws every
frozen filesystem when the job ends, whether the job returns, fails, or panics;
no path leaves a frozen filesystem or a paused sandbox behind. For the length of
the checkpoint the backend suspends its liveness check of the Cloud's runner, so
the pause does not count as a lost connection.

The pair is captured during one interval with no sandbox writer. Pausing alone
is not a persistence barrier: shared host-backed writes, host filesystem
flush/freeze, and durable publication are all required. Acceptance must
exercise ordinary writes, writable mappings, rename/unlink, and cross-volume
writers. An unresponsive runtime cannot produce a successful live checkpoint.
Report the failure; do not claim a checkpoint by silently restarting the user's
browser. If thawing or resuming fails, the manager stops the sandbox, sends the
death event, and reports the capture error together with the cleanup errors;
the backend recovers the device as a runtime loss.

Hibernate instead stops all writers, releases Gofer references, unmounts the
merged root and writable filesystems, checks and syncs both images, and
publishes the working images themselves. This path must work without a healthy
runner or user shell. A forced stop promises filesystem crash consistency;
unsynced application buffers can be lost. Neither path promises an application
transaction across files or volumes.

Publication links images instead of copying them. The staging directory of a
new generation receives hard links to images that nothing writes any more: the
working images of a stopped sandbox for hibernation, the frozen copies for a
checkpoint, and a fresh system image with the committed home image for a reset.
The manager then writes the generation's manifest, syncs the staging directory,
renames it to the generation id, syncs its parent, and atomically replaces and
syncs `current.json` and its directory. Hibernation therefore copies no image
data, a checkpoint copies each image once, and a reset writes only its new
system image. Hard links need the working and image directories on one
filesystem, which startup checks. These durable writes go through the same
atomic publication that release packaging uses
([`artifact`](../architecture/crates-and-packages.md#artifact)).

| Failure boundary | Recovery |
| --- | --- |
| Before publication | Keep the prior committed pair and the newer working pair for recovery. |
| After publication | Use the new committed pair. Never combine volumes from different generations. |
| Working state contains newer writes | Recover it before wake; do not overwrite it from an older generation. |
| Manager, backend, or host restart | Fence earlier writers, thaw surviving mounts where necessary, recover filesystems, then save or start. |
| Save fails or storage is full | Keep working data and the error; release execution resources where safe and block a stale boot. |

The immutable store retains the current and previous generations and removes
older generation directories after durable publication. Removing a directory
removes only its links: an image that a newer generation also links, such as
the home a reset carried forward, keeps its data. Staging data is removable
only after reconciliation establishes that no pending operation needs it;
failed working data is never garbage. Base directories are retained
automatically. An operator may remove an unreferenced base only with backend
and manager stopped, after checking the configured image, committed
generations, working records, and backend reset intents. This keeps a base
selected by a durable reset available even before the backend has dispatched
that reset to the manager. Snapshot sharing does not make retained generations
free; capacity monitoring includes bases, staging, working files, and retained
generations.

Cross-host storage and execution fencing are a
[scaled-deployment requirement](../backend/backend.md#deployment-and-user-ownership).
In that deployment a generation also needs a restoration path on another
worker, which must keep the same atomicity
([Multi-worker storage placement](../backend/storage.md#multi-worker-storage-placement)).

## Lifecycle and capacity

Device identity survives wake, stop, runtime loss, and reset. Concurrent first
uses join one allocation and wake. Metadata alone never creates a sandbox.

| Transition | Result |
| --- | --- |
| Off → booting → running | Recover the pinned storage, rotate the token, start a sandbox, and await the runner. |
| Running → checkpoint → running | Save a paired generation and resume the same processes. |
| Running → saving → off | End processes, save storage, and release the runtime. |
| Running → off, on runtime loss | The manager saves the working pair and reports the death; the backend ends the device's exposes and disconnects its runner. A death while the backend is saving or resetting the device is ignored; one while it boots counts toward the crash loop, and the boot fails when its runner does not connect. |
| Resetting → running | Publish a fresh system with retained home, then boot. |
| Failed boot, save, or reset | Keep the device and operation error; retry only after recovery establishes the authoritative state. |

The [idle rule](../execution/resource-lifecycle.md#idle-window) applies across
every conversation using this device. Tabs, resident services, attachments, and
passive observers do not keep it active. Maintenance does not restart the idle
clock. Stopping Cloud ends all in-sandbox browser/native state and its
[exposes](../execution/expose.md#lifetime); cleanup must not wake it or send
per-conversation release to a stopped device.

The hard lifetime cap can stop unattended jobs only after reserving admission
and rechecking active turns and other demand. A turn in flight on a
conversation that uses the Cloud, or a file transfer or user stream one of
them has open, postpones it. Otherwise new operations wait behind the
reservation, the jobs nothing attends end with the runner's connection, and
the Cloud stops once their leases are released.
Crash-loop protection stops repeated automatic boots while retaining reset; a
reset clears the recorded losses. The user's shard runs these schedules: while
the Cloud runs, its idle watch applies the idle rule, and its maintenance task
applies the lifetime cap and the checkpoint interval. Both belong to the
running phase and end with it.

Capacity is counted across users. A Cloud takes one of the backend's capacity
permits when it leaves the stopped state, to boot or to reset, and returns it
only when it is stopped again; a running Cloud that resets keeps its permit.
The permit is taken before the transition starts, so two users cannot both take
the last one. A Cloud that finds no permit free fails to start with a capacity
error; nothing queues for a permit. The count is the only Cloud state shared
across users; everything else about a user's Cloud lives in that user's shard.

| Owner | Setting | Default |
| --- | --- | --- |
| Machine manager | CPU budget / memory limit per sandbox | 2 CPUs / 2 GiB |
| Machine manager | Writable system / home initial capacity | 1 GiB / 1 GiB |
| Machine manager | `/dev/shm` / `/tmp` / `/run` size limits | 256 MiB / 256 MiB / 256 MiB |
| Backend policy | System / home maximum capacity | 16 GiB / 32 GiB |
| Backend lifecycle | Idle window / unattended-job cap | 1 hour / 24 hours |
| Backend lifecycle | Checkpoint interval | 15 minutes |
| Backend lifecycle | Runner connection after a boot or during recovery | 60 seconds |
| Backend lifecycle | Runtime losses that stop automatic boots | 3 within 10 minutes |
| Backend edge, across all users | Cloud machines booting, running, saving, or resetting | 16 |

CPU is a cgroup scheduling budget, not a virtual CPU allocation. cgroup v2
limits cover the complete sandbox, including Sentry and Gofer; temporary mounts
count toward memory and do not create extra allowances. Disable sandbox swap.
Set host `pids.max` to 1024 and the sandbox process soft/hard `RLIMIT_NPROC` to
1024. These count different things: gVisor does not map each sandbox process to
a host PID. The latter limits ordinary-user jobs, not sandbox root; CPU and
memory remain the resource boundary for sudo workloads. Reserve capacity for
the host and manager rather than treating 16 devices as safe on every VPS.

Growth changes only one working filesystem: enlarge its sparse backing file,
refresh the capacity of the loop device the sandbox holds, and grow the mounted
ext4 filesystem with `resize2fs`. It runs in the device's order, so it never
overlaps saving or reset. It is monotonic, bounded by backend policy, and
recoverable if interrupted between those steps: before the manager publishes an
image whose file is larger than its filesystem, it checks the filesystem and
grows it to the file. Record the actual filesystem capacity, read from its
superblock, not merely the requested file size, before reporting success. A
quota increase does not promise physical disk space; admission and monitoring
must handle host `ENOSPC`. A sandbox cannot grow its own backing device.

Archiving conversations or deleting project metadata never deletes Cloud data.
Account-data destruction requires its own explicit retention/deletion policy.

## System reset

The owner can reset through the backend ([Web API](../product/web-api.md#cloud))
even when the runner, shell, or system files are broken. The backend persists an
operation id, selected base, phase, and error. A retry resumes that operation,
rather than selecting a newer base.

1. Reserve device admission and hold/interrupt affected work according to
   [shared Cloud coordination](../execution/sessions-and-targets.md#coordinate-shared-cloud-activity):
   new operations on the Cloud wait for the reset, and the turns of the
   conversations that cannot work without it stop.
2. Let the runner go after a best-effort flush, which ends every command still
   running on the Cloud, those of conversations that only have it attached
   among them, and wait for the operations that held the Cloud to let go. Then
   stop the sandbox and preserve its latest home, including recoverable
   working changes newer than the last checkpoint.
3. Create an empty system layer for the selected base.
4. Durably publish the new system and preserved home as one generation carrying
   the reset operation id. The home enters the generation as a hard link to the
   saved image; only the new system image is written.
5. Announce reset to affected execution contexts, rotate the token, and boot.
   Report success only when the runner is ready.

A failed home save blocks publication and retains the working home. Before
publication recovery retains the prior pair; after publication it uses the new
pair and may retry boot. Admission remains held until an authoritative state
is established; a terminal failure is surfaced to waiting operations. Reset
never runs repair commands inside the broken sandbox. Interrupted side effects
are not automatically replayed. When the backend starts, it completes the disk
step of any reset left unfinished, announces that reset, and records the
operation as failed with a request to retry; it boots nothing until the Cloud
is next used.

Device identity, projects, and history remain unchanged. Lifecycle status and
reset phase are distinct: `resetting` may include stopping, saving, rebuilding,
or booting. Each agent node observes reset in its next persisted context.

## Isolation and joining

The isolation boundary is the gVisor sandbox, not a workspace, Docker namespace,
or hardware VM. Each user gets a separate Sentry, filesystem view, network
namespace, and resource group. Infrastructure still trusts the Linux kernel,
manager, runsc distribution, and image builder. The product supports ordinary
Linux development and browsers, not arbitrary kernel features or privileged
nested Docker. Unsupported operations return errors; they never widen
isolation.

The runtime inputs are pinned in the runtime release manifest,
`crates/machines/runtime/release.json`. The manager is built with that manifest
and refuses to start unless the configured `runsc` reports exactly the pinned
version. amd64 uses the verified upstream distribution. arm64 uses the same
source with the shipped `SECCOMP_RET_TRAP` register fix: Linux preserves the
first argument in X0 when delivering SIGSYS, while the upstream implementation
overwrites it with the syscall number.
Chrome's sandbox signal handler depends on that argument and crashes without
the fix. The ARM build has its own version and a native-versus-sandbox ABI
regression probe. It retains both gVisor isolation and Chrome's sandbox. See
the upstream
[trap handling](https://github.com/google/gvisor/blob/master/pkg/sentry/kernel/seccomp.go)
and [Chrome's signal handler](https://github.com/chromium/chromium/blob/main/sandbox/linux/seccomp-bpf-helpers/sigsys_handlers.cc).
Runtime upgrades require the same acceptance matrix, but do not change user
storage format.

The runtime profile fixes `--platform=systrap`, `--network=sandbox`, and
`--allow-suid=true`. OCI `noNewPrivileges` is false so sudo can work. The OCI
bounding set is `CHOWN`, `DAC_OVERRIDE`, `FOWNER`, `FSETID`, `KILL`, `SETGID`,
`SETUID`, `SETPCAP`, `NET_BIND_SERVICE`, `SYS_CHROOT`, and `SETFCAP` (all with
the `CAP_` prefix). The initial UID 1000 process has empty effective,
permitted, inheritable, and ambient sets; setuid sudo may acquire privileges
only within that bounding set. The profile excludes host administration, module
loading, raw devices, and network administration. Test sudo and Chrome with this
exact set. Chrome's own sandbox remains enabled. No privileged mode, host
networking, unconfined runtime seccomp, host PID namespace, or runtime socket is
exposed. Keep runsc's standard Directfs security profile; changes require a
design and acceptance update, not a per-user escape hatch.

Only the prepared root, this device's home, and explicit runtime mounts reach
Gofer. No checkout, Mac home, Docker socket, manager state, arbitrary host path,
or other user's mount is available. `/proc` and devices are sandbox views; host
sysfs and writable cgroups are not mounted. The workload cannot supply OCI
annotations, host paths, runtime flags, or network/cgroup identities.

### Networking

The manager creates a dedicated Linux network namespace and veth connection per
sandbox, through netlink. runsc builds its isolated network stack from that
namespace. There are no guest taps or per-user host port publications.

Host firewall policy lives in one fixed nftables table, which the manager
installs whole whenever it reconciles, at startup and on `reconcile`. The
table's rules are the same for every sandbox. Its only per-sandbox fact is one
element of a set: the pair of the sandbox's host interface and its source
address. Starting a sandbox adds its pair before the sandbox runs, and stopping
it removes the pair. A packet from a Cloud interface whose (interface, source
address) pair is not in the set is dropped, so a workload can neither replace
the rules nor send as another sandbox. The manager builds the table as typed
data with the `nftables` crate and applies it with `nft` in one transaction:
the netlink libraries for nftables are thinly maintained, and a text script
would be assembled from strings.

Allow outbound public traffic, replies to established connections, the exact
configured backend address/port, and the configured DNS resolver on TCP/UDP 53.
Deny other private, loopback, link-local, metadata, multicast, reserved, and
other-tenant destinations, including the host's own services. Enforce policy on
both forwarded traffic and traffic addressed to the execution host. Disable IPv6
in the initial profile so it cannot bypass IPv4 policy. Resolver configuration
uses an address reachable from the sandbox, never a container engine's loopback
resolver. Filtering is by destination address, so DNS rebinding does not grant
private-network access. Exceptions for a private backend or resolver are exact
endpoints, not whole subnets; address changes require revalidation and rule
update.

A service's public URL uses the backend's [Host expose](../execution/expose.md)
relay over the runner connection. The same connection carries files, commands,
and browser streams through normal Host access. No infrastructure transport is
an alternate route for conversation operations.

### Managed boot credential

The backend mints a managed-device token per boot and stores its hash. The
manager receives `{ backendUrl, deviceToken }` only for that boot. It decodes
the record with the runner protocol's managed-boot type, which refuses unknown
fields, and requires the backend URL to equal its configured backend URL. It
then supplies the record as a private read-only credential file on a temporary
mount. The runner accepts `--managed-boot <path>`, reads and validates the
record before connecting, and keeps its installation state under `/run/demi`.
The token never appears in process arguments, image layers, persistent working
files, OCI environment, logs, or stored generations. Only the non-secret file
path is part of the OCI process arguments. Runtime metadata containing sensitive
paths stays in an owner-restricted temporary directory. Cleanup removes it on
both failed start and stop, entry by entry, so it never deletes the contents of
a mount still attached beneath it.

A managed runner with missing or invalid boot credentials fails; it never
enters pairing. Each token permits one live connection and is scoped to that
device. Rotation rejects the previous boot's token. Sandbox root can inspect its
own runner and credential; the boundary is against other users and
infrastructure, not against the owner of that Cloud. The ordinary paired-device
registration and persistent token path remain separate, as defined by the
[runner](../execution/runner.md).

## Verification

Backend scenarios with scripted providers and a scripted machine manager
([Scenarios](../delivery/scenarios.md#system-under-test)) cover allocation
races, admission, identity, token rotation, crash loops, capacity across users,
idle policy, the lifetime cap, recovery at startup, and reset failures.

The manager runs only on Linux. Its automated tests are built with the
developer machine's own cross tools and run on Linux: inside the Lima VM on a
Mac. Tests that need root create throwaway mount and network namespaces and run
only when explicitly enabled.

Real-machine acceptance runs the exact shipped runtime, image, storage, and
network profile on Linux amd64 and arm64 without KVM, and on arm64 Lima, against
a backend with scripted models. Compilation or a Docker container cannot
substitute for these checks.
[Scenarios](../delivery/scenarios.md#real-machine-acceptance) describes how the
suite runs; a run must show the following:

| Area | Required observation |
| --- | --- |
| Identity and files | Jobs and Host file operations run as UID 1000; every conversation of a user reaches the same device; sudo works. |
| Persistence | A system package and home files survive stop and wake; home survives a reset; reset succeeds with a broken runner and with a broken system, such as disabled bash. |
| Tools | Installers run in the login shell (rustup, nvm), and later jobs find the tools they installed (cargo, node). |
| Browser | Chrome opens with its own sandbox; the live view receives frames and delivers input. |
| Checkpoint | A live checkpoint with a running browser publishes both images; the saved image contains pages a live process wrote through a writable mapping and never flushed. |
| Growth and space | Online growth records the actual capacity; a full disk fails the operation and keeps working data. |
| Recovery | A restart at each publication boundary loses neither published nor newer working data; runtime and out-of-memory loss report a death. |
| Manager recovery | After the manager is killed while a filesystem is frozen, the next start recovers through the saved namespace, and the backend reads the Cloud's files and wakes it without restarting itself. A service stop whose drain fails is completed by the stop-post recovery. |
| Network | Public HTTPS works; private, metadata, and cross-user destinations are refused; the backend and DNS paths are permitted; IPv6 is off. |
| Cleanup | No processes, mounts, loop devices, network rules, credentials, or listeners remain after success, failure, cancellation, or an injected fault. |
| Copies | On XFS with reflinks, btrfs, and ext4, a copied image has the source's content and allocates no more blocks than `cp --reflink=auto --sparse=always` does. |
| Load | Several users' Clouds run on one manager at the same time. |

Inject faults after acquiring each resource: a manager built with fault
injection aborts at a named point, and the next start must recover with nothing
left behind.

Exercise shell, native commands, provider CLI installation, and the real browser
panel through normal Host access on both Cloud and a paired device. Automated
checks use scripted models. Measure image preparation, storage setup, runsc
start, runner authentication, first command, first Chrome open, subsequent open,
first live frame, checkpoint pause, and stop separately; include cold/warm runs,
CPU architecture, memory peaks, and concurrent load. Do not turn a single
observed runtime-start duration into an end-to-end startup promise. Passing the
functional suite does not establish production capacity, concurrent-load
latency, host power-loss behavior, or multi-worker failover.
