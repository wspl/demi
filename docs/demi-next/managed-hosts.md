# Demi Next: Managed Hosts

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Delivered (M11) |
| Scope | Operator-provisioned execution targets: Firecracker provisioning, images, home persistence, lifecycle, security, joining |

## What a managed host is

A managed host is a Firecracker microVM the backend provisions on demand.
It is an operator resource: absent from the user's devices list
(`devices.kind = 'managed'`), bound to exactly one owner — a conversation
or a workspace — which every control-plane check enforces, and never reused
across owners. Managed hosts are a **deployment requirement**: the backend
needs `/dev/kvm` (bare metal or a cloud instance with nested
virtualization), and the `managedHosts` config names the kernel, the
rootfs image, the home-image store and the resource sizes. A backend
without them is not a supported deployment; support for machines without
KVM is a later design item.

Two product scenarios share one provisioning path (`sessions-and-targets.md`):

- **Session upgrade**: a hostless conversation runs its first script
  outside tinybash's subset; the backend builds the home image from the
  hostless tree (`storage.md`), provisions a host with it, binds it
  (`conversations.hostDeviceId`), hands over the shell state, and the
  whole script runs there — silently, with no context block
  (`sessions-and-targets.md`). This requires the guest to match the
  hostless layout: the guest user is `demi`, home is `/home/demi`, the
  default cwd is the home, the login environment is generated from the
  same table as the hostless `env`, and `/tmp` is placed alongside home
  at upgrade.
- **Cloud workspace**: creating a project with the **Cloud** device choice
  provisions one host for that project and creates the workspace on it;
  every conversation under the workspace executes there.

## Provisioning

The `ManagedHostProvisioner` seam (provision, wake, hibernate, checkpoint,
growHome, destroy) has one production implementation: Firecracker driven
directly as child processes, with two **launch modes** that differ only in
how the VMM process is started. Both are first-class; the operator chooses
by configuration (`DEMI_MANAGED_LAUNCH`).

- **`direct`** (the default): the backend spawns `firecracker` itself, as
  its own unprivileged user. Nothing at runtime is root. Isolation is KVM
  plus Firecracker's built-in seccomp filter. For self-hosting, single
  users and trusted teams.
- **`jailer`**: the backend asks a small **privileged helper**
  (`demi-fc-helper`, invoked through `sudo`, two verbs: start a VM, kill a
  VM, every argument whitelisted) to run Firecracker's jailer, which
  builds a per-VM chroot, mount and PID namespaces, cgroup and rlimits,
  drops to a **per-VM uid** from a slot range, and starts Firecracker
  under its seccomp allowlist. The jailer forks for the PID namespace and
  its parent exits at once, so the helper follows the pid the jailer
  records and lives as long as Firecracker does: in both modes the
  backend's child process ends when the VM does. The helper links the
  kernel, the rootfs and the working home image into the jail (the home
  shared with the backend's group, so the backend can still shrink it
  afterwards) and, once Firecracker listens, opens the jail's `run/` and
  the API socket to the backend's group; the jail must be on the same
  filesystem as the data directory for the links. A VMM escape then lands in a process that
  sees its own kernel, rootfs, home image, API socket and tap and nothing
  else — the blast radius of one tenant. For public service.
- **Guest kernel.** A minimal kernel we build (virtio net/block, ext4,
  overlayfs, tmpfs, cgroups; no modules). Boot to init is under a second.
- **Network.** A pool of taps, one per VM slot with a /30 from the managed
  subnet, created once by the **install script** (root, at install time;
  in `direct` mode the taps belong to the backend user, in `jailer` mode
  each to its slot's uid), with the egress policy as nftables rules the
  guest cannot alter, installed once for the pool: the address the runner
  is told to dial is always reachable; RFC 1918 and link-local ranges
  (`169.254.169.254` among them) are blocked; the rest of the internet is
  allowed; no inbound. The runner dials the backend's public URL like a
  user host. Operators may tighten the policy; fully-offline and allowlist
  variants are not offered, because dependency installation and
  repository cloning are the core use.
- **Resources.** vCPU and memory are VM parameters — memory is hard by
  construction (the guest sees its RAM and nothing more); disk is the home
  image's nominal size. In `jailer` mode the per-VM cgroup caps the VMM's
  own use; in `direct` mode the backend's service cgroup caps it as a
  whole.
- **Crash-loop guard.** The backend supervises each Firecracker process. N
  VM deaths within M minutes for one owner stop auto-reprovisioning and
  surface an error to the conversation.
- **Configuration.** `DEMI_MANAGED_FIRECRACKER` (the binary; unset ⇒ no
  managed hosts), `DEMI_MANAGED_KERNEL`, `DEMI_MANAGED_ROOTFS`,
  `DEMI_MANAGED_LAUNCH` (`direct`, the default, or `jailer`, which needs
  `DEMI_MANAGED_JAILER`, `DEMI_MANAGED_HELPER`, and optionally
  `DEMI_MANAGED_CHROOT_BASE`, `DEMI_MANAGED_UID_BASE`),
  `DEMI_MANAGED_VCPUS`, `DEMI_MANAGED_MEM_MIB`, `DEMI_MANAGED_HOME_MIB`
  (the nominal home size), `DEMI_MANAGED_SUBNET`, `DEMI_MANAGED_SLOTS`,
  `DEMI_MANAGED_DNS`; and `DEMI_BACKEND_PUBLIC_URL`, the URL guests dial.
  The install script (`packages/backend/scripts/install-managed-hosts.sh`,
  root, once) creates the tap pool, forwarding, NAT and the egress rules
  from the same parameters. Per VM the backend keeps a working home image,
  the API socket and the console log under `<dataDir>/firecracker/`; the
  store is `<dataDir>/homes/`. The helper is `packages/fc-helper`, a Rust
  binary; its sudoers line is the only privilege the backend user holds.
- **In a container.** Both modes run inside a Linux container on a host
  with `/dev/kvm` (`--device /dev/kvm --device /dev/net/tun`); `jailer`
  mode needs the container privileged (or the explicit capability set,
  seccomp and AppArmor unconfined, the cgroup filesystem writable). The
  container is then packaging, not a boundary; the boundary stays the
  jailer. Docker Desktop exposes no KVM.

## Images

Three block devices meet in a guest:

- **The rootfs**: one shared, read-only ext4 image per backend machine, built
  by our pipeline, fsck-clean, never mounted read-write at runtime. It is
  **preinstalled and heavy**: git, curl, build-essential, Python, Node, Bun,
  uv, Rust toolchain, ripgrep and the rest of a working developer machine,
  plus the tinyjs binary as `/demi-runner` and `/usr/bin/demi`. Its size
  costs an owner nothing — it is paged on demand and hot in the backend
  machine's page cache across every VM — and it is upgraded by shipping a new image; the
  next wake boots it with nothing to migrate.
- **The ephemeral upper**: a tmpfs (or a discarded disk) mounted as an
  overlay upper over `/` for the VM's lifetime, so `apt` and other
  system-level installs work during a session. It dies with the VM.
- **The home image**: the owner's `/home`, an ext4 image attached
  read-write. It is the only thing that persists.

The fidelity boundary reaches the agent through the standing `bash` tool
description, the same text in every state, so it reveals nothing about
where a conversation runs: system-level installs may not survive between
sessions; durable work and durable tooling go in home. The long tail
installs into home — `uv`, `nvm`, `rustup` do so already; the apt-shaped
tail goes through Linuxbrew under `/home/linuxbrew`.

## Home persistence

Only the home image persists. Hibernation kills the VM; wake boots a fresh
VM from the current rootfs and attaches the same home image. No memory
snapshots, no graceful shutdown protocol: `kill -9` on a mounted ext4 is
crash-consistent (journal replay on the next mount), the idle timeout is
minutes so the writeback window is empty at hibernation, and a single
`sync` message before the kill is cheap insurance.

**The home-image store** (`storage.md`) holds one named, mutable,
owner-bound object per owner:

```
homes/<ownerId>.ext4     overwritten in place on hibernate (temp + atomic rename)
```

It is not the content-addressed blob store: an image is one object with one
current version, streamed in and out (never a `Uint8Array` in the backend
heap). Sizing:

- Images start at a **small nominal size** and grow online: the runner
  checks the home's room after every job and once a minute (`df`), and
  when less than a tenth of it or 256 MB is free sends `home_grow` asking
  for twice the current size; the backend `truncate`s the backing file,
  tells Firecracker to rescan the drive, and answers `home_grown`, on
  which the runner runs `resize2fs`. One request is in flight at a time.
  An empty home therefore costs a few MB, not the inode tables of a large
  nominal size.
- Firecracker's virtio-block passes no discard, so a live image is a
  high-water mark. On hibernate the backend **shrinks** it offline
  (`e2fsck -f`, `resize2fs -M`, `truncate`; about 0.1 s) to retained data
  plus metadata, and re-enlarges the file to the nominal size on wake
  (about 0.01 s); the guest init runs `resize2fs` on the home device after
  the mount, so the filesystem grows back into the file at every boot.
- Hibernate starts with a `sync` message; the runner flushes the home and
  answers `sync_done { untouched }`. **Untouched** is the block layer's own
  count: the sectors written to the home device in `/proc/diskstats`,
  taken as a baseline right after the mount, unchanged at the `sync`. A
  guest that is offline or silent past the sync timeout counts as touched.
  Untouched, hibernation **skips the upload**. The runner's own state (its
  socket, command cache, job output) lives on the upper, never in the
  home, so its bookkeeping cannot touch the count.

Timing:

1. **Whenever a guest ends** — the authoritative save: VM dead, image
   consistent, shrunk, uploaded. Every path that ends a guest is this one:
   the idle rule and the hard cap, the owner archived or the workspace
   deleted, the backend closing, and a guest found still running by the
   next backend start.
2. **Periodic while running** (default 15 min, configurable) — durability
   against backend-machine disk loss during a multi-day workspace: Firecracker
   `PATCH /vm {state: Paused}` → copy the image (reflink when the backend
   machine's filesystem offers it, plain copy otherwise) → `Resumed` → upload the
   copy asynchronously. The pause window equals the copy time; the copy is
   crash-consistent and at most a writeback interval stale. Liveness
   detection exempts a host in a backend-initiated pause.

**The working image is the invariant's carrier.** A working image
(`<dataDir>/firecracker/homes/<owner>.ext4`) exists only while its guest
runs or until its save succeeded; the store holds the current home
whenever no working image exists. A save that fails — `e2fsck` refusing
the filesystem — keeps the working image where it is, logs it, and
refuses that owner's next boot until a save succeeds; nothing is ever
booted over a home the store does not hold. Each VM's run directory
(`<dataDir>/firecracker/<vmId>/`) carries a record of its process from the
moment the process exists. **Reconciliation** runs before the backend
accepts its first need: every recorded process still alive is killed and
waited for (a second guest over the same image would corrupt it, and its
tap belongs to this pool), every working image left behind is saved (it is
newer than the store), stale checkpoint copies are removed. One guest's
transitions — boot, hibernate, checkpoint, growth, destroy — run one at a
time, so a checkpoint never copies under a kill and two saves never race.

Backend-machine filesystem: xfs with reflink recommended; ext4 acceptable; btrfs a poor
fit for VM images. Guest filesystem: ext4, the one mainstream choice with
offline shrink.

Retention: images are kept. The rule for images of archived or deleted
conversations is a later design item.

## Lifecycle

```
provision ──▶ running ──▶ hibernated ──▶ running (wake) ──▶ …
                 │            home image in the store, VM killed
                 ▼
     periodic checkpoint while running
```

- **Idle rule.** Reclaim when the owner has no in-flight turn **and** the
  runner reports no running jobs (`pong.jobs`) for the idle window (~10
  min, configurable); a hard cap (24 h, configurable) reclaims a host that
  has jobs but no turns, so a forgotten daemon cannot pin a host forever. A
  workspace host counts turns across all its conversations. Zero new
  protocol messages: the runner already owns the job table.
- **Hibernate** = `sync` message, `SIGKILL` the Firecracker process,
  shrink, upload.
- **Wake** = mint a token, boot a fresh VM, attach the home image. The next
  action needing the host triggers it — a latency, not an error; idempotent
  per owner (at most one active VM per owner; concurrent triggers join the
  same wake). `demi host shell --host` on a hibernated attached host wakes it.
- **Runner is PID 1** (`init=/demi-runner`, root; the binary knows it by
  its pid, no flag): mounts `/proc`, `/sys`, `/dev`, `/run`; assembles the
  upper — a tmpfs as the upper and work directories of an overlay over the
  read-only rootfs — under `/run/newroot`, moves the kernel filesystems in
  and `pivot_root`s it to `/` (the rootfs stays reachable at `/oldroot`);
  mounts the home image (`/dev/vdb`) at `/home` and a tmpfs at `/tmp`;
  configures the network (`ip`) from the kernel command line and writes
  `/etc/resolv.conf`; reaps zombies; handles `SIGTERM` by stopping the
  runner and exiting. Every step is a command from the rootfs, not a
  primitive. Its state directory is `/var/lib/demi` on the upper — jobs
  find it through `DEMI_HOME` — and the relay socket there is mode 0666 so
  the guest user's command-mode processes can reach it. It spawns every job
  and process as the **guest user `demi`** (uid 1000, home `/home/demi`,
  `HOME`, `USER` and `PATH` set), who has **passwordless `sudo`**: `sudo apt
  install` works into the ephemeral upper, while tools that refuse to run
  as root (Linuxbrew, some package managers) work as themselves. The VM is
  single-tenant, so the user boundary is not a security boundary; it
  exists for tool compatibility and for the file ownership the model
  expects. If it exits,
  `panic=1 reboot=k` makes Firecracker exit, the backend observes the
  process death, and the next tool call takes the wake path. Nothing in the
  runner is worth preserving across a crash — the job table describes
  processes that die with it.
- **Owner archived or deleted**: the guest is hibernated — synced, killed,
  its home saved — then forgotten; the image is kept (see Retention). The
  archive flag follows the save, so a save that fails leaves the
  conversation unarchived and the error with the caller.
- **Backend closing**: every running guest is hibernated the same way
  before the process exits. A backend that dies without closing leaves its
  guests running; the next start's reconciliation kills and saves them.

## Joining

The backend pre-creates the device row (`kind: 'managed'`, token hash only)
and passes the token plaintext on the **kernel command line** (`boot_args`),
minted fresh at every spawn — provision or wake — so it rotates per VM
lifetime and never touches persistent disk. The parameters are
`demi.backend=<url>`, `demi.token=<token>`, and for the network
`demi.ip=<address/prefix>`, `demi.gw=<gateway>`, `demi.dns=<a,b>`. The
runner reads `/proc/cmdline`, keeps the token in memory only — it never
writes `runner-token` — and starts on the `hello` path. **A managed runner
without a token receives `hello_error` and never `claim_pending`** — a
managed VM never appears in a user's pairing flow.

The command line is readable by every process in the guest. That is not a
material widening: the VM is the trust boundary, and a process inside it
already sees every file and spawn the backend sends; the token's job —
keeping arbitrary internet clients from registering as devices — is intact.
One live connection per token (`runner.md`) covers the remainder.

`HostIdentity` reports `homeDir`, and that is the conversation cwd for
session-bound hosts.

## Security baseline

Uniform across deployments; sizes configurable, presence not.

1. **Isolation = KVM**, always: the guest runs its own kernel, the VMM
   under Firecracker's seccomp filter. **Plus the jailer** in `jailer`
   mode: the VMM confined to a per-VM chroot, namespaces, cgroup and uid.
   No other runtime.
2. **Resources**: vCPU, memory and disk are VM parameters; the VMM is
   cgroup-capped (per VM under the jailer, as a whole otherwise).
3. **One network rule**: no inbound; egress to the public internet only.
4. **No host mounts**: the guest sees three block devices and a tap.
5. **Credential surface**: the only secret entering a VM is its own device
   token — single-device scope, hash-stored, rotated per VM lifetime, bound
   to its owner by every control-plane check.
6. **One VM per owner, never reused**; a per-user host-count cap bounds
   provisioning.

## Verification

Tests drive the full flows — provision, bind, hibernate, wake, checkpoint,
crash-loop guard, idle rule with jobs, untouched-skip, owner-scoped authz,
auto-provision with hostless-file placement, Cloud workspace once per
project — through a fake provisioner plus a local runner everywhere. An
env-gated smoke (`real-firecracker.e2e.test.ts`) exercises real
Firecracker on Linux with `/dev/kvm` in both launch modes: the upgrade of
a hostless conversation to a guest, `sudo` into the upper, hibernate with
the shrunk image stored, wake over the same home, growth past the
reserve, destroy on archive; it prints cold-provision and wake latency.
