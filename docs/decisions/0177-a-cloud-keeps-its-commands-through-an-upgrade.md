---
status: proposed
---

# A Cloud keeps its commands through an upgrade

## Problem

Alex's agent runs `cargo build && cargo test` on Alex's Cloud, which takes ten
minutes. Six minutes in, the server's operator upgrades it. The upgrade stops
the machine manager, which saves every running Cloud before it exits: it stops
each sandbox and saves its storage. The new manager starts the Clouds again
from what was saved. The build dies with the sandbox, the command ends as lost
to the upgrade, and the agent starts the ten minutes over. A dev server Alex
left running in the Cloud is gone too.

A paired device keeps its commands through an upgrade: its runner leaves its
jobs to the runner that replaces it (0174). A Cloud cannot, for three reasons:

- The manager stops every sandbox as it exits, and the next one starts none
  of them back up as it was.
- A sandbox ends with its runner: the sandbox's init starts the runner, and the
  runner's exit ends the sandbox.
- A Cloud's runner never replaces itself. It comes from the base's
  `/opt/demi`, which the manager mounts read-only at each boot, so a new
  release reaches a Cloud only by a restart.

## Decision

A command that runs on a Cloud while the server upgrades runs on. The Cloud
does not restart: its runner replaces itself in its sandbox as a paired
device's does, takes its jobs over (0174), and reconnects. The command's
output reaches the conversation without a gap once the backend is back, and
its result is the one it would have had without the upgrade. A dev server
left running keeps its port.

The Clouds restart as now in four cases:

- An upgrade to a release that changes the manager's state format, whose
  migration runs with no sandbox running.
- A stop of the server that is no upgrade, such as `systemctl stop` or a
  reboot: the manager saves every running Cloud, as now.
- The first upgrade from a release whose manager saves the Clouds as it stops,
  and a rollback to such a release, whose `demi-server` asks for no handover.
- An upgrade whose new release does not start, which returns to the release it
  left: the release that returns cannot tell whether the manager it returns to
  takes Clouds over, so it stops the manager as any stop does.

The update screen's step that saves the running Clouds names what the
upgrade does with them, in the same row: "Keep the running Clouds" in an
upgrade that hands them over, "Save the running Clouds" in one that saves
them, so the list keeps its eight lines.

### The manager leaves its sandboxes

An upgrade tells the manager it hands over before it stops the manager: it
writes a leave request, a file in the manager's runtime directory, only when
the release it moves to reads the manager's state format as it stands, no
move of the store is due and a manager ran since the machine started, and
deletes it once the manager stopped. The release it moves to decides, since
its `demi-server` runs the move. A rollback
that moves back writes it too. A file rather than a request on the manager's
socket, so that a manager that knows none ignores it and saves its Clouds as
before, and a stop that no upgrade made never finds one.

```text
demi-server upgrade        old manager                 new manager
  |-- writes leave request ->|
  |-- stop (SIGTERM) ------>| waits for its operations,
  |                         | records the sandboxes it
  |                         | leaves, keeps its saved
  |                         | mount namespace, exits
  |-- switch, start --------------------------------->| enters the saved
  |                                                    | namespace, takes
  |                                                    | each recorded
  |                                                    | sandbox over
  |-- start backend ---> backend asks which Clouds run: every one taken over
```

- **Leaving.** On a stop that finds the request, the manager waits for the
  operations in flight, then writes a record of each sandbox that runs: its
  device, its runtime id, its network slot and the base its programs come from.
  It saves none of them, keeps the handle of its saved mount namespace, where
  their storage and network namespaces are mounted, and exits.
- **The unit.** The manager's unit takes `KillMode=process`, as containerd's
  does: systemd's stop ends the manager alone, so a sandbox in the unit's own
  cgroup, as one is with the resource limits off, outlives it. The stop-post
  recovery (`--recover`) does nothing while the record exists: the next manager
  fences and saves what the record does not name. Every start of the manager
  and its stop-post recovery first ends each process of the unit's cgroup that
  no recorded sandbox owns, and waits until it exited, before anything writes
  a disk image: what systemd's stop ended before, such as an `e2fsck` of a
  manager killed while it repaired an image, or a sandbox that `runsc` was
  killed while it made. A sandbox's Sentry and Gofer name its bundle; with the
  limits on they run in the sandbox's own cgroup, and only its
  `runsc-fd-parking` is in the unit's, which holds a descriptor of the
  Sentry's process, whose id `runsc` reports.
- **Taking over.** The new manager finds the record before its recovery. When
  the namespace's owner record names its own state directory, it enters the
  saved mount namespace and makes it its own instead of making a new one, as its
  recovery already enters it; another directory's is refused, as its recovery
  refuses it. One that cannot be entered leaves the manager in its own
  namespace, which then recovers the sandboxes as after a crash. For each
  recorded sandbox that `runsc` says runs, the device's worker takes it as
  running: its slot, its storage and a new `runsc wait`. A recorded sandbox that
  has ended is fenced and saved, as now. The network policy is installed with
  the slots of the sandboxes taken over, as a `reconcile` keeps those of the
  running ones. The pinned `runsc` manages every sandbox, also one an earlier
  `runsc` started.
- **Bases.** The bases the manager keeps after an import include each base a
  running sandbox's programs come from, so a Cloud that two upgrades pass
  keeps its `/opt/demi`.

### The Cloud's runner replaces itself

The backend that starts asks the manager which Clouds run, as it does now,
and takes each one over. Its runner, of the release before, connects through
the runner socket, which reaches the new backend once it listens. The backend
names another runner release, as it does to a paired device, and the runner:

1. downloads the release's runner through the runner socket, its only way to
   the backend, onto the Cloud's system image, and moves it to one path there,
   `/var/lib/demi/runner/demi-runner`, as an installation's, so the command
   aliases of a job a replaced runner started name a file every later release
   keeps;
2. replaces its program with it in its own process, which keeps its process
   id, so the sandbox's init sees no exit;
3. takes its jobs over from their shells (0174) and connects.

The runner's command programs follow as a paired device's do: the image's
copies are the release's before, which the artifact cache finds do not match,
and it downloads each the first time it needs it, through the socket. At the
Cloud's next boot the runner is the image's again, of the configured base, and
it removes the one it replaced itself with.

## Spike

These ran on an Ubuntu arm64 virtual machine with the pinned gVisor,
`release-20261005.0`:

- **A detached sandbox outlives its starter.** A sandbox started with `runsc
  run --detach`, the manager's way, ran on after its starter and its `runsc
  wait` were killed with `SIGKILL`.
- **One `runsc` manages another's sandbox.** Each of `release-20250106.0`,
  `release-20260105.0`, `release-20260601.0` and `release-20261026.0` started a
  sandbox that `release-20261005.0` managed, and the reverse: `state`, `list
  --format=json`, `exec`, `pause`, `resume`, `kill`, `wait` and `delete` all
  worked, also 21 months apart.
- **systemd's stop.** A unit with `KillMode=mixed`, the manager's now, ended a
  sandbox in its own cgroup (`--ignore-cgroups`, the limits off), and left one
  in a cgroup of its own. With `KillMode=process` the sandbox in the unit's
  cgroup ran on, and the unit started again beside it with systemd's
  "left-over process" warning.
- **The runner socket after the backend's restart.** A sandbox that connected
  every second to a host socket mounted as the runner socket is, with the
  manager's file access flags, reached the server that replaced the socket's
  file two seconds after the first went.
- **A program that replaces itself under init.** A process under `tini`, the
  sandbox's init, replaced its program with a file written meanwhile, kept its
  process id, and the sandbox ran on.

From the code:

- The firewall table is installed whole with the slots of the sandboxes that
  keep running, which a `reconcile` relies on already.
- `runsc` keeps its state in `/run/demi-machine-manager/runsc`, which no stop
  of the unit removes.
- The manager's state format has been 1 since it was introduced.
- A Cloud reaches the backend only through the runner socket, which the
  runner's downloads of an update do not use now.

## Prior art

- Docker's live restore keeps containers running while `dockerd` stops: on its
  shutdown it stops no container and keeps their mounts and networking, and at
  its start it asks containerd for each container's task, takes a live one
  back and cleans up a dead one
  ([`daemon.go`](https://github.com/moby/moby/blob/master/daemon/daemon.go)).
  Docker supports it across patch releases only.
- containerd's unit has `KillMode=process` and `Delegate=yes`, so its
  restart leaves the shims and their containers
  ([`containerd.service`](https://github.com/containerd/containerd/blob/main/containerd.service)).
- gVisor's `runsc` is a command over the state of its root directory, which
  any later invocation reads, as the spike's eight pairs of releases showed.

Demi departs in three ways:

- The manager hands its sandboxes over only in an upgrade that asks it to,
  and saves them on any other stop, where Docker keeps them on every stop: a
  stop that is no upgrade may be the host's shutdown, after which nothing
  takes them over, and a saved Cloud loses nothing.
- An upgrade that changes the manager's state format restarts the Clouds,
  where Docker limits live restore to patch releases.
- The workload's own agent, the runner, updates inside the sandbox it runs
  in, which a container's processes never do.

## Alternatives

- **Checkpoint and restore the sandboxes with gVisor.** `runsc checkpoint`
  saves a sandbox's memory and processes, and `runsc restore` brings them
  back. A checkpoint restores only under the same `runsc` release, writes each
  Cloud's whole memory to disk, which lengthens the upgrade with every busy
  Cloud, and cuts the sandbox's connections.
- **Restart the Clouds, as now.** Every command on a Cloud ends with every
  upgrade.
- **Upgrade once the Clouds are idle.** A dev server never ends, and the
  upgrade would wait for users' work.

## Open questions

None.
