---
status: proposed
---

# A Cloud runs on the isolation the master chooses, and Simple Isolation needs no root

## Problem

Alex wants Demi on a container platform such as Railway, where a service is a
container without root. The machine manager runs every Cloud as a gVisor
sandbox, which needs root, loop devices, network namespaces and cgroups, so
`setup` refuses the machine and the manager does not start. Alex cannot use
Demi there at all, even to drive a laptop Alex paired:

- Every deployment has Cloud, and gVisor is its only platform, with no
  fallback (`crates/machine-manager/src/lib.rs`).
- A Claude Code subscription needs the Claude Code machine, which is a gVisor
  sandbox too, so it fails wherever Clouds fail.

Alex's team trusts each other. What they need from a Cloud is that each user's
files are their own and that a mistaken `rm -rf /` in one Cloud does not take
the server down. They do not need a security boundary against a hostile
program, which is what gVisor is for.

## Decision

The master chooses how Clouds are isolated, in Settings › Cloud:

```text
Cloud
┌──────────────────────────────────────────────────────────────────┐
│ Isolation                                   [ Simple Isolation ▾ ]│
│ Each Cloud has its own system and files. Protects against         │
│ mistakes, not against a program that tries to break out.         │
│   ✓ Simple Isolation                                              │
│     gVisor                     Needs root on Linux                │
├──────────────────────────────────────────────────────────────────┤
│ Private Network Access                      [ Off ▾ ] (disabled)  │
│ Clouds share the server's network with Simple Isolation.          │
└──────────────────────────────────────────────────────────────────┘
```

- **Simple Isolation** gives each Cloud its own Ubuntu system, with `sudo`
  and `apt`, and its own home. It needs no root on the server. Inside, the
  Cloud's user is root with `/home/demi` as its home, where gVisor's is
  `demi`.
- **gVisor** is what every Cloud runs on today. It is offered only where the
  manager has what it needs; otherwise the option is disabled and its tooltip
  says what is missing, in the manager's words.
- Settings › Cloud Network becomes Settings › Cloud, and holds Isolation above
  Private Network Access. A setting that the chosen isolation cannot enforce
  stays visible, disabled, with the reason: Private Network Access, and each
  user's Cloud Storage in Settings › Users. The note the agent reads in a
  Cloud's context, that the server's private network is blocked, is left out
  for a Cloud on Simple Isolation, where it is not.
- The Claude Code machine runs on the same isolation as the Clouds.
- A new server starts on Simple Isolation. A server that upgrades to this
  release stays on gVisor.

### Switching

For example, the master switches from gVisor to Simple Isolation while Sam's
Cloud runs a build and Alex's Cloud is stopped:

- Sam's build goes on: a running Cloud keeps its isolation until it stops.
  When it stops, the manager moves it.
- The manager moves Alex's stopped Cloud right away, in the background, one
  Cloud at a time.
- A wake of a Cloud that is still to move waits for its move, and the
  Cloud's page says *Moving to Simple Isolation…* meanwhile.
- A move keeps everything: the home and the system, packages a user installed
  included. A move that fails keeps the Cloud on its old isolation with the
  error on its page; a wake goes on there, and the next wake tries again.
- If the master switches back while Alex's Cloud moves, the move finishes
  and the Cloud moves back, so it ends where the master chose last.
- Stopping or upgrading the server doesn't wait for a move: the move ends
  where it is, the Cloud stays on its old isolation, and the next start moves
  it again.
- After a switch, a toast says that stopped Clouds move now and running ones
  once they stop.
- The first switch to gVisor on a server downloads the gVisor release this
  Demi release pins, as the next section says.

### Downloading gVisor

gVisor's release is about 160 MB, and a new server starts on Simple
Isolation, so a server downloads it only when it needs it:

- `setup` no longer downloads it. It downloads it only when the machine runs
  no Simple Isolation, so that the check of the machine passes on gVisor.
- An upgrade downloads the new release's gVisor ahead of the restart only on
  a server that has downloaded one before.
- In Settings › Cloud, gVisor is offered where only its release is missing,
  with the hint *Downloads gVisor, about 160 MB, the first time*. When the
  master chooses it, the choice is saved, and the manager downloads the
  release, checks it against the pinned SHA-512, and unpacks it where it
  looks for it. Meanwhile Isolation's description says *Downloading
  gVisor…* with the megabytes received, a bar beside the menu, and Clouds
  stay on Simple Isolation. The download takes the description's place, so
  the settings below never move.
- The manager then restarts through systemd, leaving its running Clouds to
  the next manager as an upgrade does, and the next manager runs gVisor and
  moves the Clouds. The description says *Starting gVisor…* meanwhile.
- A download that fails says the error there, with a Try Again button.
- Choosing Simple Isolation again cancels a download under way, and a
  download a killed manager left is removed by the next.
- A manager that starts while gVisor is chosen and not downloaded downloads
  it, since the backend names the chosen isolation on each connection: an
  upgrade or a restart resumes the master's choice.
- A manager that systemd doesn't run, as in development, can't restart
  itself: gVisor stays unavailable, with the reason that the manager must
  restart.

The manager owns the download: `demi-server runtime`, which a setup and an
upgrade run, asks its release's manager to download into the server's
gVisor directory, so the two share one implementation.

### What each isolation does

| | Simple Isolation | gVisor |
| --- | --- | --- |
| Needs | Linux; `ptrace` | Linux; root, loop devices, network namespaces, nftables |
| A Cloud's system | A full copy of the base, about 300 MB | The base under the Cloud's own layer |
| `rm -rf /` in a Cloud | Removes that Cloud's system and home only | Same |
| A program that tries to escape | Not stopped; with root, the kernel's file permissions still keep it from other Clouds and the server's data | Stopped |
| The Cloud's user | root | `demi`, with `sudo` |
| Checkpoints every 15 minutes | No: a stop stops the processes | Yes |
| Storage limits, CPU and memory limits | No | Yes |
| Private Network rules | No: Clouds share the server's network and ports | Yes |

### The design

The backend keeps its one client of the manager. The manager gains runtimes,
each of which runs a device's processes and keeps its storage:

```text
Backend ── manager protocol ──> Machine manager
                                  ├─ Simple: proot, one directory tree per device
                                  └─ gVisor: runsc, ext4 images (today's code)
```

The manager's `hello` says which runtimes it can run and what each enforces,
and the backend names the chosen one as it names Private Network today: on
each connection and after each change. Until the backend names one, the
manager starts new devices on gVisor where it can. The master's choice is
stored in the control database; which runtime a device's storage is on, the
manager reads from the storage itself, the only one that knows it.

```rust
// crates/machine-manager-protocol
pub enum Isolation { Simple, Gvisor }

pub struct RuntimeOffer {
    pub isolation: Isolation,
    pub offer: Offer,
}

pub enum Offer {
    /// The manager runs it.
    Runs(Enforces),
    /// The manager runs it once it downloads its release.
    AfterDownload { enforces: Enforces, download: Download },
    Unavailable(Unavailable),
}

pub struct Unavailable {
    pub reason: String,
}

pub enum Download {
    Waiting,
    Downloading { received: u64, total: Option<u64> },
    Starting,
    Failed { message: String },
}

/// The settings the backend shows that a runtime may not enforce.
pub struct Enforces {
    pub checkpoints: bool,
    pub storage_limits: bool,
    pub private_network: bool,
}

pub enum MachineCall {
    // ...the calls of today, and:
    SetIsolation(SetIsolationParams),
}

// MachineResponse::Hello gains `runtimes: Vec<RuntimeOffer>`, and
// ManagerEvent::Runtimes repeats them when they change, as a download does.
// ManagerEvent::CloudStatus gains `isolation`, where the device's storage
// is, and `move_status: Option<MoveStatus>`, Moving or Failed { message }.
```

The backend reads `Enforces` and does not ask for what a runtime lacks: the
idle checkpoints of `maintenance.rs` skip a Cloud on Simple Isolation, and the
settings above are disabled. The web API adds `GET` and `PUT
/settings/cloud-isolation`, which the master's page follows on its
synchronization channel as the choice and the download change, the Users
list says whether Cloud Storage applies, and a Cloud's status carries its
move.

### Simple Isolation

A device's storage is a directory tree:

```text
<state>/simple/<device>/
  record.json   the base the system is pinned to, the last reset, the user
  process.json  the running proot, while one runs
  system/       /: a copy of the base it is pinned to, then the user's changes
  home/         /home
  shm/          /dev/shm, emptied at each boot
  proot/        proot's own files, emptied at each boot
```

- **Boot.** The manager copies, never binds, what the device must not change
  into `system`: the configured base's `/opt/demi` (the runner and its
  programs), the boot record, and the resolver and hosts files. A bound
  directory is writable through proot, so `rm -rf /` would reach it. It then
  starts `proot --kill-on-exit -0 -r system`, in a session of its own,
  binding `home`, `shm`, and the host's `/dev`, `/proc` and `/sys`, which
  `rm -rf /` cannot empty: their files are the kernel's. The first process is
  `tini`, which starts the runner, as on gVisor.
- **The user.** proot fakes the owner of every file as the user it runs as,
  so `sudo` under a faked `demi` finds its configuration owned by `demi` and
  refuses. The Cloud's processes therefore run as a faked root, with `HOME`
  `/home/demi`.
- **The runner's connection.** The runner reaches the backend on an abstract
  Unix socket named after the backend's socket file,
  `demi-runners:<path of the file>`, which the backend listens on beside the
  file. An abstract socket has no file that `rm -rf /` can remove.
- **Users.** A manager with root runs each device as a user of its own,
  allocated from a range and recorded with the device. The device's
  `system`, `home` and `shm` are that user's, in a directory of root's that
  only that user's group may pass through, and the manager's and the
  backend's files stay root's. A manager without root runs every device as
  its own user.
- **Chrome.** `demi browser` already starts Chrome with `--no-sandbox` on
  Linux, which Simple Isolation needs, since Chrome's own sandbox needs
  namespaces proot does not give.
- **Stop.** The manager signals the Cloud's first process, `SIGTERM` and then
  `SIGKILL`. When it ends, `--kill-on-exit` makes proot end every process it
  traced, those that left the session included, and exit. Ending proot
  instead would leave them running: a tracer's death detaches its tracees.
  Then it syncs the tree's filesystem.
- **The fence.** A proot that dies unasked leaves its processes running,
  untraced, so that the paths they open are the host's. The manager ends
  what remains of a Cloud as soon as its proot exits, and again before it
  boots it or moves it. With root, that is every process of the device's
  user. Without root, every Cloud runs as the manager's own user, and the
  manager ends every process whose program, as the kernel names it in
  `/proc/<pid>/exe`, is a file under the device's directory: proot runs
  every program through its loader, which it writes into the device's
  `proot/`. The runner removes every `DEMI_*` name from the environment of
  the programs it starts, so an environment variable would not find them.
  A program that gets past proot and runs one of the host's escapes this
  fence, which is one more way Simple Isolation protects against mistakes
  only.
- **Reset.** The manager replaces `system` with a fresh copy of the
  configured base and keeps `home`.
- **Manager restarts.** The manager records each running device's proot pid,
  start time and boot in the device's `process.json`, beside its files, so a
  manager without root needs no runtime directory. A manager that leaves its
  Clouds to the next one, as an upgrade does, names them in its record of
  what it left, and the next takes over each whose process still matches,
  through a pidfd, as it takes over gVisor sandboxes today. The next manager
  fences every other device that has a `process.json`, such as the Clouds of
  a manager that crashed.
- **A host without gVisor.** The manager checks gVisor's requirements as it
  starts. When one is missing, such as root, it logs why and runs every Cloud
  on Simple Isolation; a Cloud whose storage is gVisor's fails to start, with
  that reason, until gVisor is available again. A host that runs neither
  stops the manager, with both reasons.
- **proot.** Demi pins a commit of Termux's fork of proot, which is
  maintained and tested on amd64 and arm64 Linux, and builds it statically
  for both architectures from its source, with its one library, talloc, and
  glibc linked in, in a pinned Ubuntu container: proot publishes no builds.
  proot-me's own releases, 5.5.0 included, crash when the runner starts a
  program in the background, and on arm64 check `test -w` and `test -x`
  against the host's paths, which breaks package scripts. The server release
  ships it as `bin/proot`, with the two source archives it was built from in
  `sources/`, which each release also publishes as assets, since proot's GPL
  and talloc's LGPL ask for the source to accompany the program.

### A move between runtimes

- **gVisor to Simple.** The manager mounts the Cloud's system image as an
  overlay over its pinned base, read-only, and copies the merged tree into
  `system`, and the home image's files into `home`.
- **Simple to gVisor.** The manager makes a system image holding what
  `system` changes in its pinned base: files that differ or are new, and a
  whiteout for each file the base has and `system` lacks. `home` goes into a
  home image.
- The manager checks for room before it starts, since both copies exist until
  the move ends, and removes the old storage once the new one is recorded. A
  move cut short in between leaves the storage on both runtimes; the manager
  keeps the one recorded last, the move's result, and removes the other.
- A Simple Isolation tree records no owners: all of a device's files are its
  user's. A move to gVisor gives a changed system file its owner in the base,
  a new one its directory's owner, and the home to `demi`: whoever writes in a
  directory only its owner may write in owns what it writes there, such as
  the log the runner keeps in its own directory.
- A move from gVisor reads the Cloud's images as gVisor does, through loop
  devices, so only a manager that can run gVisor moves a Cloud off it. On a
  server that can no longer run gVisor, a Cloud on gVisor stays there, unable
  to start, until gVisor runs again.
- The manager keeps the bases Simple Isolation's systems are pinned to, as it
  keeps those of gVisor's generations, since a move to gVisor compares a
  system with its base.

## Spike

| Question | What ran | Result |
| --- | --- | --- |
| Does proot run on a container platform? | A probe service on Railway | `ptrace` is allowed on every host it landed on. User namespaces, mounts, loop devices, cgroups and `/dev/fuse` are not, so bubblewrap and gVisor cannot run |
| Does the Cloud base work under proot? | proot as an ordinary user on Railway (x86_64) and in Docker (arm64), with the Ubuntu 26.04 base and Demi's Cloud image | Fake root, `sudo`, `apt-get install`, `git`, Python, listening ports and the runner work. Chrome needs `--no-sandbox`. proot 5.1 and 5.4 fail on the base; 5.5.0 passes |
| How much slower is it? | The same file workloads with and without proot | 1.5 to 4 times slower on file-heavy work, close to native on CPU work |
| What does `rm -rf /` reach? | `rm -rf --no-preserve-root /` inside proot, natively on arm64, with the server's files owned by the same user | Only the device's tree and what was bound into it. Files outside it, the same user's, were untouched. Under x86 emulation on macOS it also removed files outside, which shows proot is no boundary against a program that gets past it |
| Can proot ship with Demi? | A static build of Termux's proot with glibc and talloc | 1.1 MB, no dynamic dependencies. The fork uses glibc's interfaces, so it does not build with musl |
| Does a Cloud run the product's commands under proot? | The runner in a Simple Isolation Cloud, in Docker on arm64, running commands and `apt-get install` for an agent | Under proot-me's 5.5.0, proot crashed at the first command, and `test -w` failed in package scripts. Under Termux's fork, both work |
| Can the runner reach the backend without a bound file? | A process inside proot connecting to an abstract socket outside | It connects |
| How does a Cloud stop whole? | Background processes, some started with `setsid`, then the first process ended, or proot ended | Ending the first process ends every traced process and proot. Ending proot leaves every one of them running |
| Can the user be `demi`, with `sudo`? | proot with `-i 1000:1000`, then `sudo -n id` | `sudo` refuses: its files appear owned by uid 1000. As a faked root, `sudo` and `apt` work |
| Does an environment variable find a Cloud's processes? | `DEMI_CLOUD` in the first process's environment, then `/proc/<pid>/environ` read by the same user | A descendant holds it; one started with `env -u DEMI_CLOUD` does not, and neither does a program the runner starts, since the runner removes `DEMI_*` names |
| Does a Cloud's program name its device? | `/proc/<pid>/exe` of the programs a Cloud's runner started, read by the same user | Each names the loader in the device's `proot/`, also after proot was killed |
| Does a base unpack without root? | `bsdtar -xpf --xattrs` of the Cloud image as an ordinary user | It succeeds without a warning |
| How large is a device's system? | The 0.1.6001 arm64 Cloud image, unpacked | About 300 MB |

## Prior art

- [OpenHands](https://docs.openhands.dev/usage/architecture/runtime) has one
  `Runtime` interface with implementations for Docker, a local process,
  Modal, E2B, Daytona and others, chosen by configuration.
- [ComputeSDK](https://www.computesdk.com/docs/reference/adding-a-provider)
  asks each provider for create, get and destroy, with snapshots only where a
  provider has them. Demi's runtimes declare what they enforce in the same
  spirit, since checkpoints, limits and network rules exist on one runtime
  only.
- [proot-distro](https://github.com/termux/proot-distro), from Termux, runs
  full Linux distributions as one directory tree each, without root, on
  Android. Demi's Simple Isolation is the same arrangement.

Where Demi departs: both projects pick a runtime when a sandbox is made, and
neither moves one between runtimes. Demi moves a Cloud, since a Cloud is a
user's long-lived machine, not a disposable sandbox.

## Alternatives

- **The runtimes in the backend, the manager as one of them.** The manager
  already owns every device's processes and storage, and its protocol is
  already the interface the backend needs, so the split would only copy it.
- **bubblewrap.** It needs user namespaces and mounts, which Railway refuses.
- **fakechroot.** It rewrites paths through `LD_PRELOAD`, which Demi's
  statically linked runner and programs ignore.
- **Claude Code as a process of the backend.** It would free Claude Code
  from the manager, but it needs a second way to run an isolated process, and
  it could never use gVisor. The Claude Code machine on the chosen runtime
  needs none of that.
- **A switch that applies to new Clouds only.** Existing users' files would
  stay on a runtime the server no longer runs.

## Open questions

None.
