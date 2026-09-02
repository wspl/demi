# Demi Next: Execution Layer Review (2026-09)

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Review record — decisions taken during the M7 spike and confirmed or revised in owner review on 2026-09-02; ready to fold into `demi-next.md` |
| Scope | Managed-host provisioning, host lifecycle, the execution model (hostless execution, the command system, the shell), the runner, the RPC data carrier, the host access model, the `demi` command surface |
| Working state | Nothing committed. Spike artifacts live outside the repo (Lima VM `fc`, `/opt/fc`, scratchpad; the QuickJS shell spike under `scratchpad/shell`). |

This document records what was investigated, what was measured, and what
was decided in one continuous review. Each item states the **decision** in
final-state wording (liftable into `demi-next.md` as-is), then the
**evidence** it rests on, then the **consequences** for existing design and
code. Rejected alternatives appear only where the rejection itself carries
information the final design depends on.

---

## 0. Decision index

| # | Area | Decision |
|---|---|---|
| 1 | Provisioning | Managed hosts are **Firecracker microVMs**, not runsc containers |
| 2 | Lifecycle | **Home-only persistence.** No memory snapshots. Hibernate = `kill -9`; wake = fresh VM + attach home |
| 3 | Lifecycle | Idle rule **Z**: reclaim when no turns *and* no running processes; hard cap on "processes but no turns" |
| 4 | Lifecycle | Runner is **PID 1**; crash = VM death = wake path; crash-loop guard lives in the backend |
| 5 | Lifecycle | Device token via **kernel command line**, minted per spawn; managed runners never enter the claim flow |
| 6 | Lifecycle | Home image is a **named mutable object** bound to its owner, never deleted; not a content-addressed blob |
| 7 | Lifecycle | Upload on **hibernate + periodic checkpoint** (pause → copy → resume → async upload) |
| 8 | Security | Firecracker under **jailer**; egress policy **c** (internet allowed, private ranges + metadata endpoint + backend internals blocked, no inbound) |
| 9 | Execution model | **Drop just-bash and the portable command set.** Real hosts run real bash; a conversation without a host runs **demi commands only**, against a backend-side filesystem |
| 10 | Execution model | `demi` is the **QuickJS shell plus the loader**; one binary with two entry modes (runner / CLI); `command-bridge` deleted |
| 11 | Execution model | No execution target → hostless demi execution; the **first non-demi command auto-provisions** a managed host and the backend places the hostless files on it |
| 12 | Wire | **Bytes rule**: protocols carry references, never bulk bytes; bulk moves over HTTP |
| 13 | Wire | Runner-side **tee**: full output lands on the target as artifact files; the wire carries a bounded view; artifacts are **not uploaded** on hibernate |
| 14 | Carrier | **zod (schema) + MessagePack (wire)**; the protocol stays TS↔TS, no code generation; MessagePack is a shell primitive |
| 15 | Runner | **Rust writes the shell only**; runner logic is JS on the shell |
| 16 | Wire | Control-priority channel layer **deferred**; recorded as a risk with a stated precondition |
| 17 | Access | `prev` / `switch` / `release` removed; **per-conversation host grants**, user-granted only, auto-grant on switch |
| 18 | Commands | `demi host list` / `current` / `shell --id <hostId> <shell_content>`; **groups navigate, leaves execute** |
| 19 | Agent protocol | `AgentServer` / `AgentClient` split **unchanged**; only the shell-runtime backing re-homes to the runner |
| 20 | Commands | One command tree, defined in the backend; every command is **`rpc`** (runs in the backend) or **`runtime`** (an ESM module shipped to the target and run there) |
| 21 | Commands | **Command ABI**: a runtime module is `export default (ctx) => …` and sees only `ctx` (Host fs, cwd, env, stdio, parsed args) |
| 22 | Commands | The **loader** is a pure-JS library (manifest source + Host + optional rpc transport → hash-cached dispatch); the runner, the CLI and the hostless backend path are its embedders |
| 23 | Shell | The shell provides IO primitives, the event loop, the Web-platform globals and every **byte-level path** (base64, MessagePack, UTF-8, tee); JS handles control flow only |
| 24 | Lifecycle | Home-only persistence stands: the VM mounts an **ephemeral upper** for its lifetime; the **preinstalled rootfs** is ours to maintain; the long tail installs into home |
| 25 | Lifecycle | One device token accepts **one live connection**; a second `hello` with the same token is rejected and logged |
| 26 | Lifecycle | Home-image retention rule **deferred** |

---

## 1. Environment and method

**Environment.** All measurements were taken on an Apple M5 Max (macOS
26.5.2). runsc ran inside OrbStack's Linux VM (kernel 7.0.14-orbstack,
aarch64, cgroup v2, no `/dev/kvm`). Firecracker ran inside a Lima VM
(`vmType: vz`, `nestedVirtualization: true`, Ubuntu 24.04, kernel 6.8,
aarch64) — nested virtualization is available on M3+ under macOS 15+; UTM
supports it, VMware Fusion and Parallels do not; Lima was chosen because it
is CLI-driven.

**Evidence discipline.** Every number below is from a nested-virtualization
environment and is therefore **pessimistic**: positive results transfer to
bare metal; negative results were only accepted after a control experiment.
Two findings were reversed by controls during the review — a cgroup memory
limit that appeared ignored (the cause was swap, not gVisor) and a Bun
startup cost attributed to disk I/O (the cause was runtime initialisation,
not I/O). Both are recorded where they occur.

A second environment produced the wake-time decomposition and the shell
spike in §6: Apple M3 Pro (macOS 26.0), Lima 2.1.0 (`vmType: vz`,
`nestedVirtualization: true`, Ubuntu 24.04, kernel 6.8), Firecracker
v1.16.1 with the CI `vmlinux-6.1.155` kernel, 2 vCPU / 1 GB guests, a
debootstrap noble rootfs, and a Bun stub standing in for the backend's
runner socket. Its numbers are of the same order as the first environment's
and are reported separately where they differ.

**x86_64 is unverified.** No bare-metal or x86_64 run happened. The
architecture-sensitive item (syscall compatibility) became moot with
Firecracker; the remaining numbers are expected to improve on bare metal.

---

## 2. Provisioning: Firecracker replaces runsc

### Decision

Managed hosts are Firecracker microVMs, driven directly by the backend as
child processes under `jailer`. The `ManagedHostProvisioner` seam is
unchanged. There is no runsc implementation.

### Evidence — runsc

The design's "nests inside a containerised backend with zero host grants"
claim was tested with a privilege matrix inside a Docker container:

| Grant | Result |
|---|---|
| none | gofer cannot start (`fork/exec /proc/self/exe: EPERM`) |
| `seccomp=unconfined` only | sandbox process cannot start |
| `CAP_SYS_ADMIN` only | past gofer, fails at sandbox sync file |
| `CAP_SYS_CHROOT` instead of `SYS_ADMIN` | fails |
| **`CAP_SYS_ADMIN` + `seccomp=unconfined`** | **works** |
| `apparmor=unconfined` | not needed |

Two further requirements surfaced: the backend container must vacate its
root cgroup before runsc can enable `cgroup.subtree_control` (cgroup v2 "no
internal processes" rule), and `memory.max` alone is not a hard limit —
with 17 GB of host swap a 64 MB cgroup accepted a 200 MB allocation
(`memory.events: max 1408, oom_kill 0`); `memory.swap.max=0` was required to
get `oom_kill 1`. gVisor's rootless mode was disqualified for this product:
cgroup configuration errors are ignored, Netstack is unsupported (host
network only), and `runsc create` is unsupported.

Independent corroboration: running gVisor nested is documented to require
privileged mode, which "adds one layer while removing several (seccomp, all
capability restrictions, device isolation)". None of the production sandbox
providers surveyed nest (E2B: Firecracker; Modal: gVisor; Daytona: plain
containers; Anthropic `sandbox-runtime`: bubblewrap / Seatbelt).

### Evidence — Firecracker

| Measurement | Value |
|---|---|
| Nested KVM on Apple Silicon | works (`/dev/kvm`, KVM API v12) |
| Guest boot to init, official minimal kernel (`vmlinux-6.18.44`, 19 MB) | **0.65–0.70 s** |
| Guest boot to init, Ubuntu generic kernel (57 MB) | 1.42–1.45 s |
| Snapshot create (512 MB VM) | 0.40 s |
| Snapshot restore | **12 ms**, running processes survive (counter continued, in-memory secret intact) |
| Memory limit | hard by construction: 256 MB VM sees 236 MB; a 400 MB write OOMs the guest; host memory 480 → 482 MB |
| Snapshot memory file | full VM RAM, **not sparse** (512 MB on disk) |

### Why Firecracker

- **Real guest kernel.** The syscall-compatibility risk that gVisor carries
  for a coding agent running arbitrary toolchains does not exist.
- **Memory is hard by construction.** The entire cgroup delegation / swap
  discipline runsc needed disappears.
- **The KVM requirement lands on the operator.** Managed hosts are an
  operator resource gated by `managedHosts` configuration (absent → "managed
  hosts not configured"); self-hosters pair their own machine (a user host)
  and never need a managed host. The "runs anywhere" property of
  self-hosting is unaffected.
- **Runtime grant is `/dev/kvm`.** Setup-time root is needed by `jailer`
  (see §8); that privilege is confined to a short setup phase in an audited
  binary rather than held by the long-running backend.

### Consequences

- `demi-next.md` § Provisioning and § Security baseline describe runsc and
  are invalidated wholesale. § Execution targets keeps its shape.
- Costs accepted: the guest kernel is now ours to build and patch; rootfs
  images are block devices (ext4 files) rather than directories; no local
  development without a KVM-capable machine (Lima solves this on M3+).
- Firecracker exposes only virtio-net / virtio-block / virtio-vsock — no
  virtio-fs. Guest filesystems are opaque images to the host.

---

## 3. Lifecycle: home-only persistence

### Decision

A managed host persists **only its home block image**. Hibernation kills
the VM (`SIGKILL` on the Firecracker process); wake boots a fresh VM from
the shared read-only rootfs and attaches the same home image. No memory
snapshots, no graceful shutdown protocol.

Three statements make home-only persistence hold (Decision 24):

- **The rootfs is preinstalled and ours to maintain.** Its size costs a
  conversation nothing: it is one shared read-only file per backend host,
  paged in on demand and hot in the host page cache across every VM. A
  heavy base image is therefore the cheap place to put toolchains.
- **The VM mounts an ephemeral upper layer** (tmpfs or a discarded disk)
  over the rootfs for its lifetime, so `apt` and other system-level installs
  work during the session and vanish at hibernation. A persistent overlay
  was rejected: an upper layer carrying a dpkg database and overwritten
  system files makes the base image un-upgradable, while home-only lets the
  next wake boot a patched rootfs with nothing to migrate.
- **The long tail installs into home.** `uv`, `nvm`, `rustup` already live
  there; the apt-shaped tail goes through a home-resident package manager
  (Linuxbrew under `/home/linuxbrew`). The context block injected on
  switch states that system installs are lost on hibernate and names the
  home-resident alternatives.

### Evidence

- Home-only restore verified: VM-1 writes to `/home`; VM-1 is killed with
  no snapshot; VM-2 boots with the rootfs mounted `ro` and the same home
  image attached; the random payload, directory tree and a 50 MB file read
  back intact. Wake wall clock 1.16 s excluding runner start-up.
- `kill -9` is safe: after the kill the image reports `Filesystem state:
  clean`; the next VM mounts it without fsck (journal replay); a file
  written without `fsync` exists **with empty content**, a file written with
  `fsync` is intact. The realistic loss window is the data-writeback interval
  (`dirty_expire_centisecs`, default 30 s), not the 5 s journal commit. The
  idle timeout is minutes, so the window is empty by construction at
  hibernation; a single `sync` message before the kill is cheap insurance
  against an activity-detection false negative.
- **Memory snapshots solve a problem this product does not have.** Design
  invariant 1 ("sessions live in the backend") means nothing in VM memory is
  worth preserving; the design already lets running processes block
  hibernation, so "processes die" never applies to legitimate work.

### Constraint discovered

The shared rootfs must be **fsck-clean when built and never mounted
read-write at runtime**. A dirty ext4 journal (left by a `reboot -f`) makes a
read-only mount fail with `Unable to mount root fs`; `e2fsck -fy` cures it.
The build pipeline owns this.

### Storage cost and reclamation

Firecracker's virtio-block does **not** pass discard through (`fstrim:
ioctl failed: Not supported`), so a home image is a high-water mark: 300 MB
written then deleted inside the guest still occupies 369 MB on the host.
Reclamation is offline and needs no mount and no privilege:

| Step | Result |
|---|---|
| `e2fsck -f` + `resize2fs -M` + `truncate` | 369 MB → 69 MB in **0.11 s** |
| Re-grow on wake: `truncate` to nominal + `resize2fs` | **0.01 s** |

Reclaimed size is *retained data + metadata*, not zero. Nominal image size
should be **8 GB**: mkfs metadata is 33 MB at 1 GB, 66 MB at 2 GB, **69 MB at
8 GB**, 261 MB at 32 GB — ext4's inode density makes 8 GB nearly free
relative to 2 GB.

### Home image store (Decision 6)

The home image is **not** stored in the content-addressed `BlobStore`. It is
a named, mutable, owner-bound object:

```
homes/<ownerId>.ext4        overwritten in place on hibernate (temp + atomic rename)
```

The owner is the conversation (session upgrade) or the workspace (Cloud
workspace) — the two are mutually exclusive on a conversation today. Because
conversations are archive-only, the image is effectively permanent. Content
addressing would have manufactured superseded versions, orphans, a
pointer-switch crash window, and a retention policy; none of those exist
under this model. `BlobStore` is untouched (it stays put/get for
attachments and transcript media, which are permanent by design). A small
separate store with **streaming** `write(ownerId, stream)` / `read(ownerId)`
is required: 100 MB+ images must not pass through the backend heap as a
`Uint8Array`.

Two cost controls follow from per-conversation ownership: the image starts
at a small nominal size and grows online (`truncate` + in-guest `resize2fs`)
so an empty home costs a few MB rather than the 69 MB of inode tables an
8 GB nominal image carries; and hibernation skips the upload when the
runner reports the home untouched. The retention rule for images of
archived or deleted conversations is deferred (Decision 26).

### Upload timing (Decision 7)

Two timings answer two questions:

1. **On hibernate** — the authoritative save: consistent (VM dead), shrunk.
2. **Periodic while running** (default 15 min, configurable) — durability
   against backend-host disk loss during a multi-day active workspace. The
   design's own metadata is Litestream-replicated with seconds of RPO; user
   project files must not be weaker.

Mechanism, needing no guest cooperation: Firecracker `PATCH /vm
{state:Paused}` → copy `home.ext4` → `Resumed` → upload the copy
asynchronously. The pause window equals the copy time. The checkpoint is ≤30
s stale (guest page cache) which is acceptable for a backup. Liveness
detection must **exempt** a host in a backend-initiated pause.

Host filesystem: **xfs with reflink** recommended (copy becomes
instantaneous); ext4 acceptable (0.1–0.4 s copy for a few hundred MB on
NVMe); btrfs is a poor fit for VM images (`chattr +C` disables the CoW that
would have helped). The code must use reflink opportunistically, never
require it. Guest filesystem stays **ext4** — the only mainstream choice with
offline shrink, which reclamation depends on; xfs cannot shrink at all.

### Idle rule (Decision 3)

| Option | Rule | Rejected because |
|---|---|---|
| X (design as written) | no turns and no processes → reclaim | a forgotten daemon pins a host forever |
| Y | no turns → reclaim regardless of processes | kills a deliberately running dev server |
| **Z** | X, plus a hard cap (e.g. 24 h) on "processes but no turns" | — |

Mechanism: the runner owns the job table (real bash runs on the target), so
its `pong` carries the active job count; the backend's existing idle sweep
combines that with session activity. Zero new protocol messages. The idle
timeout (~10 min) and the cap are configuration.

### Runner supervision (Decision 4)

The runner is PID 1 (`init=/demi-runner`). It performs init duties: mount
`/proc` `/sys` `/tmp` `/home`, configure the network from the kernel
command line, **reap zombies** (it spawns many bash processes), handle
`SIGTERM`. If it exits, `panic=1 reboot=k` makes Firecracker exit, the
backend's process handle observes it, and the next tool call takes the wake
path. There is no runner state worth preserving across a crash (the job
table describes processes that die with it; the connection is re-established),
so in-VM supervision buys nothing. A minimal-init-plus-restart design was
rejected for the same reason (orphaned bash processes would have to be killed
anyway); systemd in the guest was rejected for image size and boot time. The
backend holds the crash-loop guard: N deaths in M minutes stops
auto-reprovisioning and surfaces an error to the session.

### Device token (Decision 5)

The per-VM device token travels on the **kernel command line**
(`boot_args`), read from `/proc/cmdline`. It is minted fresh at every spawn
(provision or wake), so it rotates per VM lifetime for free and never touches
persistent disk (a token in `home.ext4` would leak with every backup and has
the wrong lifetime). Firecracker's MMDS was rejected: it introduces a
network-before-token boot ordering and shares the `169.254.169.254` address
the egress policy blocks. The runner already needs the command line for the
backend URL and network configuration. Protocol: unchanged `hello
{deviceToken}`; the backend resolves the token hash to a `kind: 'managed'`
device. **A managed runner without a token receives `hello_error` and never
`claim_pending`** — a managed VM must never appear in a user's pairing flow.

The command line is readable by every process in the guest, including the
untrusted code the agent runs. That is not a material widening: the VM is
the trust boundary, and a process inside it already sees every file and
spawn request the backend sends. The token's job — keeping arbitrary
internet clients from registering as devices — is intact. One hygiene rule
covers the remainder (Decision 25): **a token holds at most one live
connection; a second `hello` carrying it is rejected and logged.** This
applies to user hosts as well. Managed runners dial the same public backend
URL as user hosts; there is no private-network path to carve out of the
egress policy.

---

## 4. Execution model: real bash on real machines, demi-only without one

### Decision (9, 10, 11, 20–23)

The just-bash interpreter role of `@demicodes/shell`, the portable command
set, and `command-bridge` are removed. On every real execution target — a
user host, a managed microVM, the local machine — commands run in **real
bash on the target**, spawned by the runner with the session's cwd and env
passed explicitly (both are already tracked in backend session state). The
agent never initiates a target switch.

A conversation with **no execution target runs demi commands only**: the
backend parses the tool call with a small demi-only parser, executes the
command in-process against a filesystem backed by the conversation's store,
and refuses anything else with an upgrade message. The **first non-demi
command auto-provisions** a managed host, and the backend writes the
hostless files into its home — no model-driven migration.

**The command system** (Decisions 20–23):

- One command tree, defined once in the backend. Every command is `rpc` —
  its implementation runs in the backend — or `runtime` — its implementation
  is an ESM module shipped to the target and run there. `demi file *` and
  future `demi search` are `runtime`; `todo`, `agent`, `host` are `rpc`.
- A runtime module is `export default (ctx) => …` and sees only `ctx`: the
  Host fs interface, cwd, env, stdin/stdout and the parsed arguments. No
  runtime builtins. This is the public **command ABI**: the same module runs
  on the shell against a real filesystem, in the backend against the
  hostless store, and in tests.
- The **loader** is a pure-JS library: given a manifest source, a Host and an
  optional rpc transport, it caches the command table and modules by hash and
  dispatches calls — runtime modules locally, rpc commands as typed messages
  carrying the already-parsed arguments. The runner embeds it with the backend
  connection as manifest source and a disk cache; the standalone `demi` CLI
  embeds it with a configured source; the hostless backend path embeds it
  in-process. A third party needs the library, a Host and a source — no
  runner, no shell.
- `demi` on a target is the **shell plus the loader**: the same binary as the
  runner, selected by entry mode. It reads the runner's manifest cache from
  disk and asks the runner over a local UDS only on a miss; the CLI holds no
  credential. Session attribution for rpc commands is by ids injected into
  the bash environment, as the deleted bridge did with `DEMI_SHELL_ID`.

### Why a hostless mode stays, and why it is small

A large share of conversations only write a file or run a query. Provisioning
a microVM for those pays a boot, minutes of resident memory and a persistent
image for work that never needed a machine. The hostless mode keeps that
work free, and it is small because everything the model does there is a
`demi` command: the parser tokenizes one simple command (quotes, escapes),
supports heredocs (`<<EOF`, `<<'EOF'`, `<<-EOF`, `<<<`) — `demi file create`
and `demi file patch` take their content on stdin and the repository already
emits heredocs for them — and sequences joined by newline, `;` or `&&`.
Pipes, redirections, variables, substitutions, globs and any non-`demi` first
word are refused. It does not pretend to be bash, so the bash-behavior
divergence catalogue has nothing to describe. The tool description in the
hostless state says so, so the model reaches for `demi` rather than `cat`.

### The pipeline finding

`cmd1 | cmd2` between two real processes on the same target transits the
backend: just-bash implements `|` in memory. The interpreter hands the
previous stage's output to `hostSpawn` as a latin1 string
(`environment.ts:868`, `:913`), collects the real process's full stdout
(`:937`) and returns it as the next stage's input (`:962`). Bytes cross the
wire up and back down. This is inherent to "interpreter in the backend,
processes on the target"; the entire `bash-behavior.md` divergence catalogue
exists because pipes were not pipes. With real bash on the target the pipe is
an OS pipe and **zero bytes cross the wire**.

### What just-bash actually did on a real host

`shouldPreferHostSpawn` already routes everything outside
`IN_PROCESS_PORTABLE_COMMANDS` to the real host, with `bash`/`sh`/`sleep`
pinned to real processes. On a real host just-bash was (1) the parser and
orchestrator, (2) the in-process host for the registered `demi` commands,
(3) the capture / limit / artifact / job wrapper around them. (1) is replaced
by real bash; (2) by runtime modules run by the loader; (3) moves to the
runner.

### Why runtime modules rather than a second implementation

Verified against imports: `demi file *` depends only on `host.fs`. Running it
in the backend for every host would cost two wire round trips per file
operation; implementing it once per language would leave two copies of
every file command to keep in sync. Shipping the implementation as a module
gives one implementation and zero round trips, and it is what makes the
toolkit story real: the command tree, the module format and the ABI are the
contract a third party builds on.

### Consequences

- **Deletion scope**: `packages/just-bash` (the fork submodule), the
  interpreter and portable-command parts of `packages/shell`,
  `docs/bash-behavior.md`, `host-local/command-bridge`, the virtual-prev tar
  pipe, the 64 MB capture `SIGKILL` (its rationale — the backend holding
  whole outputs in memory — disappears with the tee). `packages/host-virtual`
  is reduced to the store-backed Host the hostless mode needs.
  `IN_PROCESS_PORTABLE_COMMANDS` is `echo`, `printf`, `pwd`, `alias`,
  `unalias`, `history`, `help`, `time` — all bash builtins, nothing to
  replace.
- The runner bundle today is 4.5 MB, of which just-bash is 2.0 MB and its
  dependencies (yaml, domino, fast-xml-parser, …) another 2.1 MB; the runner,
  protocol and host-local proper are about 50 KB. The deletion removes the
  module-evaluation cost measured in §6.
- The job model to re-implement on the runner is small: one foreground
  process plus background jobs, and `extractSimpleBackgroundCommand`
  accepts only a single simple command with `&` (no pipelines, operators,
  redirections or assignments).
- **Windows user hosts** have no bash. There is no Windows bash story today
  either (the only `win32` branch in `host-local` is a data-directory path),
  so this is not a regression, but "a user host needs bash" becomes an
  explicit statement.
- The runner grows (job table, tee, UDS relay) but gains no power in the
  backend → device direction; the "thin, auditable runner" trust argument
  survives.
- **`AgentServer` / `AgentClient` are unaffected** (Decision 19). The
  browser-hosted-server idea was proposed (`9246a4c4`) and reversed in the
  same design round (`947a9a78`): a page-hosted server dies with the page
  and cannot continue an in-flight turn; the gateway rematerialises sessions
  with no page open; the browser is always a client. `packages/agent/src`
  has no reference to virtual-host or just-bash. `AgentClient` is a 476-line
  client state machine (transcript mirror, revision resync, steer/abort
  waiters) that `web-ui` is built on. The only shell coupling is precisely
  located — `AgentServerOptions.shell`, `LiveSession.environmentForHost` /
  `createEnvironment`, and the `shell_write` / `shell_output` /
  `shell_write_result` frames (`binding.ts:365–413`, `live-session.ts:186`,
  `client.ts:178, 264, 431`). The frames keep their semantics; their backing
  moves from an in-backend `BashEnvironment` to the runner's job table.

---

## 5. Wire: the bytes rule and the tee

### Decision (12, 13)

> Protocols carry references, never bulk bytes.

- File reads and writes happen on the target (`demi file *` is local).
- The runner **tees** each command's full output to artifact files on the
  target; the wire carries only the bounded observation view (1 MB class).
- When the backend needs full bytes it fetches them by reference over
  **HTTP**, never over the WebSocket.
- Artifacts stay on the target and are **not uploaded** on hibernate. The
  full output of a past command is reachable while its host is online; a
  hibernated managed host is woken on demand to serve it, an offline user
  host shows "full output is on an offline host". The value of full output
  decays fast, and uploading it at hibernation would move the very bytes the
  rule exists to keep off the wire.
- Browser-bound media becomes a reference plus a `GET` endpoint;
  `transcript_reset` / `transcript_patch` no longer inline bytes.
- Cross-host transfer (`demi host shell --id A "tar cz …" | tar xz`) must
  relay through the backend — both runners dial out and cannot reach each
  other — and does so over a brokered one-shot HTTP transfer (source `PUT`,
  destination `GET`, backend pipes the two streams), never over the WebSocket.

### Evidence — what the wire carries today

- **Backend → browser inlines media.** `sync_transcript` →
  `sendTranscriptReset` (`binding.ts:225, 305`) → `cloneBlocks` is
  `structuredClone` (keeps `Uint8Array`) → `stringifyPortableJson`
  (`websocket-transport.ts:33`). `externalizeBlockMedia` is called only from
  the two persistence paths (`session-store.ts:44`,
  `conversation-store.ts:54`), never before sending. The M6 attachment-ref
  design is one-directional: browser → backend sends refs; backend → browser
  ships every image as base64 on every reconnect.
- **Giant stdout.** The runner pumps every chunk with no cap
  (`host-rpc-server.ts:96–102`); `RemoteSpawn.chunks` is an unbounded array
  (`remote-host.ts:178`); the backend enforces a 64 MB ceiling by dropping
  the raw capture and `SIGKILL`ing (`environment-output.ts:66`) — so up to
  64 MB crosses the control socket and is then discarded. The artifact is
  then written **back** over the wire via `host.fs.writeFile`
  (`command-artifact-store.ts:44–47`; ≤1 MB text view + ≤16 MB binary) even
  though the runner already held those bytes. The design bounded memory,
  not channel occupancy.
- **Portable codec.** `stringifyPortableJson` / `parsePortableJson`
  (`utils/json.ts`, 125 lines) keep JSON as the carrier and wrap three
  non-JSON types in tagged envelopes: `{__demiUint8Array, base64}`,
  `{__demiDate, iso}`, `{__demiBigInt, value}` — recovering the originals via
  the replacer's holder because `toJSON` runs first. Seven call sites: the
  runner wire (byte-heavy hot path), the agent WebSocket and stdio transports,
  `conversation-store` (media already externalised before encoding),
  `host-store`, `local-store`, `runner/state`. Only the runner wire is
  byte-dense on a live connection.

### Recorded risk (Decision 16)

WebSocket is one ordered stream with no multiplexing: a message's fragments
cannot interleave with another message's, and application-level `ping` /
`pong` are data frames. Two concrete failure modes exist even with the bytes
rule: a `ping` queued behind a view chunk producing a false "runner offline
mid-turn", and a user `abort` queued behind a large `transcript_patch`. The
fix, if needed, is a channel layer beneath the carrier: stream-id framing,
≤16 KB chunks, strict control-first priority, and `bufferedAmount`
backpressure at a low-water mark (the last is what makes the first three
real). **Deferred** on the stated precondition that patches stay small;
worst-case wait with a 1 MB view is ~100 ms at 10 MB/s and ~1 s at 1 MB/s.
To be re-examined if a large patch source appears.

---

## 6. Data carrier and runtime

### Decision (14)

The RPC carrier is **zod as the schema language (unchanged) and MessagePack
as the wire (native bytes)**. Both ends of the runner protocol are
TypeScript — the backend on Bun, the runner as JS on the shell — so the
schemas are shared as-is and **no code generation exists**. Presence
semantics are correct by construction (zod is required-by-default,
`.optional()` is explicit). MessagePack encoding and decoding is a shell
primitive, not JS.

### Evidence

- **zod is a runtime value validator, not an IDL.** zod 4.4.3 has no bytes
  type: `z.instanceof(Uint8Array)` and `z.custom<Uint8Array>` validate a real
  `Uint8Array` but cannot be exported to JSON Schema, and `z.base64()`
  validates a *string*. The schemas describe the decoded in-memory shape;
  the wire carries whatever the codec chooses. With MessagePack that codec
  carries bytes natively and the schemas need no envelope types.
- **What validation the RPC boundary actually uses.** Zero refinements in
  `runner-protocol/schemas.ts` and `agent/protocol/schemas.ts`; all 13
  refinements in the codebase are on Web API bodies (`connections` 7,
  `workspaces` 4, `devices` 2). Structural validation is inherent in a
  generated decoder; the only thing to carry over explicitly is field
  presence. `protovalidate` exists if a refinement is ever needed on the wire.
- **Cross-language generation was tried and is unnecessary.** Walking the
  zod AST (`.def`, `.meta`, `z.globalRegistry`) and the JSON-Schema route
  (`z.toJSONSchema` → `cargo typify`, 1948 lines of Rust) both work well
  enough for discriminated unions and poorly for plain unions and custom
  types; either would be a generator to maintain against zod internals. With
  both protocol ends in TypeScript the question does not arise. The
  exercise did expose two protocol defects that stand regardless of carrier:
  the fs layer is untyped (`fs_call.args: unknown[]`, `fs_result.result:
  unknown` — no drift protection where errno / stat fidelity matters most),
  and `fs_result` shares `type` across its ok / error branches. Fixes:
  per-op discriminated fs messages; nest ok / error one level down.
- **Protobuf and Bebop rejected.** Both would add a second schema language
  beside zod (which stays for TS↔TS boundaries and HTTP bodies). With the
  runner in JS there is no cross-language protocol boundary at all: the
  only Rust↔JS surface is the shell's primitive API, which knows nothing
  about the protocol. The Claude Code stdio, Firecracker API and Codex
  WebSocket remain third-party protocols.

### Runtime (Decision 15): Rust writes the shell, the runner is JS

The runner, the `demi` CLI and the hostless path all execute JS. On a
target, that JS runs on a **shell**: a Rust binary embedding QuickJS
(rquickjs) that provides the IO primitives (fs with errno fidelity, spawn
with pipes and tee, TCP/UDS/TLS, WebSocket, HTTP client, timers), the event
loop, the Web-platform globals QuickJS lacks (`TextEncoder`, `TextDecoder`,
`atob`, `btoa`, `URL`, …) and every byte-level path (base64, MessagePack,
UTF-8, tee). JS on the shell handles control flow only. Runtime command
modules never see the shell API — only the loader does.

**Why not a Bun or Node binary.** Cold start of each runtime inside a fresh
microVM, first execution (nested virtualization, which is the realistic
deployment: Firecracker needs `/dev/kvm`, and cloud instances provide it
through nested virtualization, not bare metal):

| Runtime | First exec | Second exec | Size |
|---|---|---|---|
| Rust hello | **0.030 s** | 0.000 s | 453 KB |
| QuickJS shell hello | **0.12 s** | 0.01 s | 1.4 MB |
| Go hello | **0.120 s** | 0.020 s | 1.6 MB |
| Bun hello | 0.89–1.67 s | 0.10 s | 86–99 MB |
| Node hello | 1.940 s | — | node 49 MB |
| Bun, real runner | 4.95 s | 1.56 s | 103 MB |

**Where the Bun runner's time goes.** A second measurement run (M3 Pro, Lima
vz, Firecracker 1.16.1, 2 vCPU, 1 GB, a stub backend answering `hello` in
1 ms) reproduced the 6.24 s "runner online" figure as 6.3 s and decomposed
it:

| Segment | Time | Evidence |
|---|---|---|
| Bun runtime baseline | 0.10 s | hello, second exec |
| First executable mapping of the 100 MB binary | ~3.4 s | same file 4.95 s then 1.56 s; a tmpfs copy 4.6 s then 1.6 s; host `drop_caches` makes no difference; equal page-fault counts, 310 extra major faults on the first exec |
| Evaluating the 4.5 MB bundle | ~1.45 s | second exec minus baseline; `--bytecode --minify` no gain (1.54 s); 90 % of the bundle is just-bash and its dependencies |
| Runner `main` → online | 0.8–1.0 s | LocalHost construction 0.17, token read 0.17, TCP + upgrade 0.29, `hello` round trip 0.05, config write 0.26 |

Neither page-cache warming nor bytecode caching touches the first row; it
scales with the number of executable pages touched (hello: 0.9–1.5 s,
runner: 3.4 s, shell: 0.1 s). The mechanism consistent with every
observation is per-page kernel work on the first executable mapping of a
file (arm64 instruction-cache maintenance), amplified under nested
virtualization; no perf profile confirmed it, and x86 nested behaviour is
unverified. The second row is deleted with just-bash (§4). The conclusion
is that the binary must be small; LLRT would also qualify on this axis but
carries a Node-compatibility surface, an experimental label and gaps
(WebSocket, UDS) that would need Rust modules anyway — at which point the
shell is the same work with a smaller, fully owned API.

**Shell feasibility spike.** A 150-line rquickjs shell exposing `readFile`,
`writeFile`, `spawnTee`, `tcpGet` and native base64, running a 386 KB ESM
bundle of the real `runner-protocol` codec plus zod 4, in the same guest:

| Measurement | Shell | Reference |
|---|---|---|
| Binary | 1.45 MB (glibc, dynamic) | Bun 103 MB |
| hello, first / second exec | 0.12 s / 0.01 s | Bun 1.61 s / 0.10 s |
| Protocol bundle module evaluation, first / second | 149 ms / 37 ms | — |
| zod encode of a byte-free message ×200 | 1 ms | — |
| zod decode of a `spawn` frame ×200 | 8–10 ms | — |
| tee 100 MB to a file with a 1 MB view | 83–96 MB/s | `head \| cat > file` in the guest: 78 MB/s |
| native base64, 16 KB | 300 MB/s | — |
| `bytesToBase64` from `@demicodes/utils` (pure JS) | **2.8 MB/s** | native on Bun |
| TCP to the stub, first / second | 41 ms / 5 ms | — |

zod 4 runs unmodified in QuickJS; validation cost is negligible; the tee
primitive saturates the guest's pipe and disk. The one order-of-magnitude
loss is a pure-JS byte loop without a JIT, which fixes the rule in
Decision 23: byte-level work is a shell primitive, never JS. QuickJS lacks
the Web-platform globals and an event loop; the spike supplied the former
with a prelude and ran synchronously — the event loop is the largest piece
of shell design work.

Guest-image consequences met on the way (relevant to M10): Bun's default
target is glibc and fails on alpine (musl) with "not found"; the shell must
be built as a static musl binary. The runner locates state via `homedir()`,
so PID-1 init must set `HOME`.

---

## 7. Host access model and the `demi host` command

### Decision (17, 18)

`prev`, `switch` and `release` are removed. A conversation has a current
execution target (existing `hostDeviceId` / `workspaceId`) and a **grant
set** (`conversation_host_grants`) of other hosts it may reach. Grants are
created **only by the user** in the web UI; switching the target in the
picker **automatically grants the departed host**; the agent can never grant.
The backend checks the grant table before dispatching a cross-host spawn.
Being a grant target does not pin a host against idle reclamation; `shell
--id` on a hibernated managed host wakes it.

```
demi host                                        help for the group
demi host list                                   granted hosts (id, kind, online; current marked)
demi host current                                current execution target
demi host shell --id <hostId> <shell_content>    run a shell string in the host's real bash; byte-faithful stdio
```

`<shell_content>` is executed by the remote host's `bash -c`, so pipes,
redirections and globs apply remotely; it starts in the host's default cwd.

### Why

With virtual gone and auto-provision on first tool call, the agent never
switches; with release reduced to "drop the pointer" it does nothing; and
`prev` was a temporal slot standing in for an **authorization** relation. The
M6 per-type disposal table (virtual nothing / managed hibernate / user revoke)
had a real defect under shared ownership — releasing a Cloud-workspace host
would have hibernated it under other sessions — and collapses entirely:
reclamation is the idle rule's job alone. The grant model is the
"per-session grants — the direction of the deleted workspace allowlist" that
`demi-next.md` § Prior art already names as the response to the trust
asymmetry.

### Command rule

> **Group nodes navigate; leaf nodes execute.** A `Command` with
> `subcommands` must not have `run`; invoking a group bare prints the group's
> help.

Audit of the current tree: `demi`, `file`, `todo`, the child-side `agent`
node and `host prev` comply; **`agent` (parent) violates** — its group-level
`run` (`subagent/commands.ts:67`) is the spawn, and there is no `spawn`
leaf (leaves are `steer/abort/resume/list/show`); **`host` violates**
(`host-command.ts:34`, bare status). The dispatcher also violates the rule:
exhausting argv on a run-less group throws `requires a subcommand`
(`command.ts`), so the rule needs one dispatcher change (return `help:
true`). Fixes: add `demi agent spawn [--profile] [--description] [prompt]`
(touches `docs/subagent.md` §§ 42/53/60/71, the `demi agent:` error strings
and the `resume` description; nothing in `coding-agent` teaches the bare
form — the model learns from the tree's own help); `host` is rewritten under
the grant model anyway. All three are changes to the one command tree the
loader serves.

---

## 8. Security posture of managed hosts

### Decision (8)

- **jailer is mandatory.** It confines the *Firecracker process* (not the
  guest — KVM does that): per-VM chroot, unprivileged uid/gid, mount / PID
  namespaces, cgroup, rlimits; Firecracker installs its own seccomp
  allowlist. Defence in depth for a guest → VMM escape. Two consequences:
  kernel, rootfs and the home image must be visible inside the chroot
  (hardlink / bind), and **invoking jailer requires root** — a small
  privileged provisioner spawns it while the backend proper stays
  unprivileged. This corrects the earlier "only `/dev/kvm`" framing: the
  privilege is real but confined to a millisecond setup phase in an audited
  binary, not held by the long-running backend as runsc required.
- **Egress policy c.** Internet allowed; **blocked**: RFC1918 and link-local
  ranges, `169.254.169.254`, the backend's own internal network; **no
  inbound**. Enforced host-side by per-tap nftables rules the guest cannot
  alter; operator-tightenable. Fully-offline and allowlist variants were
  rejected because dependency installation and repository cloning are the
  core use of a coding agent.
- The six-item runsc baseline in `demi-next.md` no longer applies.

---

## 9. What remains to test

Pulled out per the "decide first, then test" process:

1. The full deletion scope (`shell` interpreter vs Host-contract parts; any
   agent-layer path that bypasses the shell to reach virtual).
2. Shell primitives not covered by the spike: the event loop, WebSocket and
   TLS client, UDS, spawn stdin and pty, fs errno fidelity, static musl
   linking.
3. Runner spawning real `bash -c` with env-injected session ids; a `demi
   todo` round trip over the UDS relay; a runtime module served from the
   manifest cache.
4. `cmd1 | cmd2` on a managed host with zero wire bytes observed.
5. Cold provision latency (new owner, no home yet) and wake latency with the
   shell-based runner (projection: kernel boot plus ~0.5 s).
6. Firecracker under jailer; chroot layout for kernel / rootfs / home.
7. Periodic checkpoint: pause window with and without reflink.
8. x86_64 confirmation of the Firecracker and first-exec numbers.
9. If the channel layer is revisited: control-message latency and ping
   stability under a saturated stream.

## 10. Effects on `demi-next.md` and the roadmap

- Rewrite: § Virtual execution (becomes hostless demi execution),
  § Provisioning, § Security baseline, § Lifecycle, § Session and host model
  (prev slot → grants), § The `demi host` command, § Attachments (browser
  media by reference both ways), § Changes to existing packages, § Verified
  facts / Storage pluggability (blob store vs home store), the M2 / M6 / M7
  roadmap rows and their verification rows. A new section describes the
  command system: command kinds, the command ABI, the loader, the manifest
  and its cache.
- Milestone order changes: **shell + loader + command manifest + hostless
  execution + just-bash removal** is a new milestone that precedes M7. M6's
  prev slot, `switch`, `release` and the tar-pipe migration are removed by
  it; the owner accepted that rework.
- `docs/bash-behavior.md`, `docs/command-bridge.md` and the portable-command
  parts of `docs/package-boundaries.md` become obsolete.

## Sources consulted

gVisor rootless mode documentation; Firecracker design and getting-started
documentation; Lima `nestedVirtualization` (vz) and UTM 4.6 release notes;
sandbox landscape surveys (E2B, Modal, Daytona, Anthropic `sandbox-runtime`);
Protobuf-ES 2.0 announcement; Buffa; prost; `cargo-typify`; Bebop; `buf
breaking` documentation; rquickjs and QuickJS-ng; LLRT (AWS Low Latency
Runtime) documentation.
