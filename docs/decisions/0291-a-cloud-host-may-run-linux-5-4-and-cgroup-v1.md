---
status: proposed
---

# A Cloud host may run Linux 5.4 and cgroup v1

## Problem

An operator sets up Demi Cloud on a server that runs Debian 10, with Linux
5.4, booted with `systemd.unified_cgroup_hierarchy=0` for an older Docker,
so it has the cgroup v1 hierarchies and no unified one. The setup guide asks
for Linux 5.14 and cgroup v2, and the manager agrees: `--check-host` fails
for the missing cgroup v2 controllers, and with the limits off the first
Cloud still fails, since the manager calls what Linux 5.4 does not have.
The operator's only way is another server, though the kernel isolates a
gVisor sandbox as well as a newer one does.

What 5.4 lacks, each found by running the manager on it:

| The manager uses | From | Where |
| --- | --- | --- |
| `LOOP_CONFIGURE`, to attach a disk image | 5.8 | `linux/loopdev.rs` |
| `statx`'s mount-root flag, to tell a mount point | 5.8 | `linux/mount.rs` |
| `openat2`, to open a base's executable beneath its root | 5.6 | `storage/base.rs` |
| `cgroup.kill`, to fence a sandbox | 5.14 | `sandbox/cgroup.rs` |
| The cgroup v2 root | — | `sandbox/cgroup.rs` |

Debian 10's own nftables, 0.9.0, also refuses the manager's firewall table,
and its error names no reason.

## Decision

The manager runs on Linux 5.4 or later, with cgroup v2 or v1, and nothing a
Cloud does changes with the kernel: the same isolation, limits, disks and
firewall.

* **Each call the older kernel lacks has the way before it,** taken when the
  kernel refuses the newer call, as other tools take it:
  * A disk attaches with `LOOP_SET_FD`, then `LOOP_SET_STATUS64` sets
    auto-clear, as util-linux's `losetup` does; a device whose status cannot
    be set is detached again. The first refusal of `LOOP_CONFIGURE` makes
    every later attach go the older way at once, once the older way attached:
    a newer kernel refuses with the same error a backing file it cannot take.
  * A mount point is one that the calling thread's mount table,
    `/proc/thread-self/mountinfo`, lists: a thread that entered the host's
    namespace reads the host's, where `/proc/self` would name the manager's.
  * A base's executable opens one component at a time, following no link and
    no `..`, which is stricter than `openat2`'s `RESOLVE_BENEATH`: a base
    names its executables by their real paths, so neither way meets a link.
* **cgroup v1 is a hierarchy as v2 is.** runsc makes a sandbox's cgroup from
  the bundle's `cgroupsPath` on either: under the v2 root, or in each of the
  v1 `cpu`, `memory` and `pids` hierarchies. The manager reads which one the
  host mounts (`/sys/fs/cgroup/cgroup.controllers` is v2's) and requires the
  three controllers there: listed by the v2 root, or mounted as v1
  hierarchies. It enables the controllers for the sandboxes on v2; a v1
  hierarchy has its controller at once. The limits are the same numbers in
  each. On v1 the memory limit holds swap too only on a kernel booted with
  `swapaccount=1`, which the setup guide says.
* **A fence kills without `cgroup.kill`.** runsc makes a sandbox's cgroup in
  every v1 hierarchy the host mounts, not only in those of the limits, and
  one it stopped making has some of them. A fence takes the sandbox's
  cgroup in each hierarchy that has one. Where `cgroup.kill` is missing, on
  a kernel before 5.14 or on v1, the manager sends `SIGKILL` to each process
  those cgroups list until they list none, within the deadline a fence by
  `cgroup.kill` waits, then removes each.
* **The firewall needs nftables 0.9.3 or later.** `--check-host` and the
  manager's start read `nft --version` and refuse an older one by name
  ("nftables 0.9.0 cannot read the firewall's JSON; 0.9.3 or later reads
  it"), then check the manager's own table with `nft -c`, so a kernel that
  lacks what the table uses refuses it with nft's words before any Cloud
  starts. The installation guide says that Debian 10 installs `nftables`
  from `buster-backports`, then runs `setup` with `--no-packages`.
* **The extractor needs bsdtar 3.3.3 or later**, which reads the base's
  zstd; Debian 10's is 3.3.3.

Every message that named cgroup v2 says "cgroup controllers" now; the
setup and installation guides say Linux 5.4, cgroup v2 or v1, and the two
tools' versions.

## Spike

An Ubuntu 20.04 VM with Linux 5.4, booted with
`systemd.unified_cgroup_hierarchy=0 swapaccount=1`, ran the manager built
from this change:

* `--check-host` passes, and the manager starts ready.
* A Cloud wakes, runs a command, writes its home,
  resolves names, reaches a public address, saves a checkpoint, hibernates
  and wakes again with its data kept.
* The v1 limits hold: with one CPU, a busy loop gets 3,005 ms of CPU in 3 s;
  the memory, memory-and-swap and PID limits are set; a hibernation leaves
  no cgroup behind.
* A manager killed with `SIGKILL` leaves its namespace; the next one
  recovers it, saving the Cloud's disks, and leaves one handle.
* A sandbox whose runsc record is gone and whose cgroups are in all 16
  hierarchies of the host is fenced at the next start, which leaves none.
* nftables 0.9.0 refuses the table ("Empty string is not allowed", for its
  concatenated set type), and the version check names it; 0.9.3 and 0.9.6
  load it. With the `nft_fib` modules kept from loading, the check reports
  nft's "Could not process rule".
* bsdtar 3.3.3 extracts the base image.

On Linux 7.0 with cgroup v2, a Cloud wakes, hibernates and is recovered
after the manager's `SIGKILL` as before.

Not run: a kernel of another vendor's build on x86-64, a host where Docker's
rules share the firewall, and a whole run with a backend.

## Prior art

* util-linux attaches with `LOOP_CONFIGURE` and falls back to `LOOP_SET_FD`
  and `LOOP_SET_STATUS64` on a kernel that refuses it
  ([`lib/loopdev.c`](https://github.com/util-linux/util-linux/blob/master/lib/loopdev.c)).
* runc has a manager for each hierarchy
  ([`libcontainer/cgroups`](https://github.com/opencontainers/runc/tree/main/libcontainer/cgroups)),
  and kills a container without `cgroup.kill` by signaling each process its
  cgroup lists. It freezes the cgroup first, since a container's processes
  fork; a sandbox's are gVisor's Sentry and Gofer, which do not, so the
  manager signals until the list is empty instead.
* gVisor's runsc reads both hierarchies
  ([`runsc/cgroup`](https://github.com/google/gvisor/tree/master/runsc/cgroup)).
* libpathrs opens a path one component at a time where `openat2` is missing
  ([libpathrs](https://github.com/cyphar/libpathrs)); it follows links that
  stay beneath, where the manager follows none, since a base has none on the
  way to its executables.

## Alternatives

* **Keep 5.14 and cgroup v2.** The operator moves to another server for
  calls the kernel answers in another way.
* **Run on v1 with the limits off only.** The manager would need the
  fallbacks anyway, and the Clouds would have no limits on a host that
  offers them.
* **Bundle nftables and bsdtar.** The manager uses the system's tools on
  every distribution; one that ships them old installs newer ones, as
  Debian does from its backports.

## Open questions

None.
