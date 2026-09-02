# Demi Next: Managed Hosts

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (M10) |
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

The `ManagedHostProvisioner` seam (provision, wake, hibernate, destroy) has
one production implementation: Firecracker driven directly as child
processes.

- **jailer.** Every Firecracker process runs under Firecracker's jailer:
  per-VM chroot, unprivileged uid/gid, mount and PID namespaces, cgroup,
  rlimits, plus Firecracker's own seccomp allowlist. It confines the VMM
  process; KVM confines the guest. Invoking jailer needs root, so a small
  **privileged provisioner helper** spawns it and drops back; the backend
  proper stays unprivileged and holds only `/dev/kvm`. Kernel, rootfs and
  the home image are hardlinked or bind-mounted into the chroot.
- **Guest kernel.** A minimal kernel we build (virtio net/block/vsock,
  ext4, overlayfs, tmpfs, cgroups; no modules). Boot to init is under a
  second.
- **Network.** One tap per VM. Egress policy, enforced on the backend machine by
  per-tap nftables rules the guest cannot alter: internet allowed; RFC 1918
  and link-local ranges, `169.254.169.254`, and the backend's own internal
  network blocked; no inbound. The runner dials the backend's public URL like
  a user host, so no private-network exception exists. Operators may tighten
  the policy; fully-offline and allowlist variants are not offered, because
  dependency installation and repository cloning are the core use.
- **Resources.** vCPU and memory are VM parameters — memory is hard by
  construction (the guest sees its RAM and nothing more); the jailer cgroup
  caps the VMM's own use; disk is the home image's nominal size.
- **Crash-loop guard.** The backend supervises each Firecracker process. N
  VM deaths within M minutes for one owner stop auto-reprovisioning and
  surface an error to the conversation.

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

- Images start at a **small nominal size** and grow online: the backend
  `truncate`s the backing file and the runner runs `resize2fs` in the
  guest when usage nears the cap. An empty home therefore costs a few MB,
  not the inode tables of a large nominal size.
- Firecracker's virtio-block passes no discard, so a live image is a
  high-water mark. On hibernate the backend **shrinks** it offline
  (`e2fsck -f`, `resize2fs -M`, `truncate`; about 0.1 s) to retained data
  plus metadata, and re-grows it on wake (about 0.01 s).
- If the runner reports the home untouched since wake, hibernation
  **skips the upload**.

Timing:

1. **On hibernate** — the authoritative save: VM dead, image consistent,
   shrunk, uploaded.
2. **Periodic while running** (default 15 min, configurable) — durability
   against backend-machine disk loss during a multi-day workspace: Firecracker
   `PATCH /vm {state: Paused}` → copy the image (reflink when the backend
   machine's filesystem offers it, plain copy otherwise) → `Resumed` → upload the
   copy asynchronously. The pause window equals the copy time; the copy is
   crash-consistent and at most a writeback interval stale. Liveness
   detection exempts a host in a backend-initiated pause.

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
  same wake). `demi host shell --id` on a hibernated granted host wakes it.
- **Runner is PID 1** (`init=/demi-runner`, root): mounts `/proc`, `/sys`,
  `/tmp`, the upper and `/home`, configures the network from the kernel
  command line, reaps zombies, handles `SIGTERM`. It spawns every job and
  process as the **guest user `demi`** (uid 1000, home `/home/demi`,
  `HOME` and `PATH` set), who has **passwordless `sudo`**: `sudo apt
  install` works into the ephemeral upper, while tools that refuse to run
  as root (Linuxbrew, some package managers) work as themselves. The VM is
  single-tenant, so the user boundary is not a security boundary; it
  exists for tool compatibility and for the file ownership the model
  expects. If it exits,
  `panic=1 reboot=k` makes Firecracker exit, the backend observes the
  process death, and the next tool call takes the wake path. Nothing in the
  runner is worth preserving across a crash — the job table describes
  processes that die with it.
- **Owner archived or deleted**: VM destroyed, image kept (see Retention).

## Joining

The backend pre-creates the device row (`kind: 'managed'`, token hash only)
and passes the token plaintext on the **kernel command line** (`boot_args`),
minted fresh at every spawn — provision or wake — so it rotates per VM
lifetime and never touches persistent disk. The runner reads
`/proc/cmdline` (it needs the command line for the backend URL and network
configuration anyway) and starts on the `hello` path. **A managed runner
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

1. **Isolation = KVM + jailer.** The guest runs its own kernel; the VMM runs
   jailed. No fallback runtime.
2. **Resources**: vCPU, memory and disk are VM parameters; the VMM is
   cgroup-capped.
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
env-gated smoke exercises real Firecracker on Linux with `/dev/kvm`,
recording cold-provision and wake latency.
