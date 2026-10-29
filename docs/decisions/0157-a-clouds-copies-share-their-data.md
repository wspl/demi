---
status: proposed
---

# A Cloud's copies share their data, and no large write fills the disk

## Problem

Alex runs Demi on a 38 GB server whose disk is ext4. Bo's Cloud holds 3.4 GB
of data, and the machine manager keeps three full copies of it: the working
copy the running Cloud writes, its latest save, and the save before it,
which a start falls back to when the latest is damaged. Every 15 minutes a
checkpoint copies the working copy into a new save, and drops the oldest
only after, so for its 17 seconds the Cloud holds four copies, 13 GB. Each
wake copies the latest save into a new working copy, 4 seconds for 1.7 GB.

The copies are mostly the same bytes: on that server, the two saves of each
Cloud differed in no block at all. The manager already clones a file by
sharing its extents (`FICLONE`) where the filesystem can, but ext4 cannot,
so every copy takes its full size.

One afternoon such a checkpoint ran while the server prepared an update
ahead, unpacking the next release and importing its image, and the disk
filled: the backend's search index could not be written for half an hour,
so messages sent then are missing from search, and the system log lost its
lines. Nothing checks for room before any of these writes.

## Decision

A Cloud's working copy and its saves share the data they have in common,
so a save or a wake copies nothing, and a save that changed nothing costs
nothing. Every large write first makes sure the server's disk has room for
it, and is refused rather than filling it.

Nothing changes on any page but one message: an update whose preparation
lacks room fails at its step, and Settings › Updates shows it as it shows a
failed step today:

```text
✕ Install Image
  Not enough disk space: needs 1.2 GB more.
```

**The store shares extents.** The manager's data directory,
`/opt/demi/data/cloud`, with its bases, saves and working copies, is on a
filesystem that clones a file by sharing its extents:

* The host's own, when it can, which a clone of a small file there tells.
* Otherwise an XFS filesystem with `reflink=1` in a sparse file beside it,
  `/opt/demi/data/cloud.xfs`, as Incus makes a storage pool without a disk of
  its own:
  * The file is as large as the host's filesystem and takes only what XFS
    holds.
  * A line in `/etc/fstab` mounts it at the store from a loop device with
    `discard`, so what XFS frees goes back to the host, and `nofail`, so a
    failure never holds the machine's boot. The line also orders the
    manager's service after the mount and makes it require it
    (`x-systemd.before`, `x-systemd.required-by`), so no manager of any
    release starts on the directory beneath. The line belongs to no release,
    so a rollback keeps it.
  * The manager refuses to start on a store whose file exists but is not
    mounted, which would show none of its Clouds, and turns direct I/O on for
    the loop device, so the file's pages are cached once.
* A store that is a filesystem of its own, such as a disk mounted there,
  stays as it is.

`demi-server setup` makes the store, and installs `xfsprogs` with the other
packages. With it, the copies the manager already makes are clones: a
wake's working copy and a checkpoint's save. The previous save stays, as
today, since it holds only what changed since it.

**Moving an existing store in.** The update to the first release that has
this moves the store after its switch, as the new release's manager starts,
while the manager is stopped and every Cloud is saved, so no working copy
exists. The update screen shows the move as the progress of its step Start
the Cloud Manager.

1. Before anything stops, the preparation requires `mkfs.xfs`.
2. The move holds one item twice at a time. Before it begins, it needs the
   room of its largest item, a base or a save, with the reserve below to
   spare, and before each item, that item's: what an earlier run copied of
   it goes first. While the host lacks the room, the largest previous save
   goes. When none is left, the step fails, and the server stays stopped on
   the update screen, which names the shortfall.
3. The XFS file is made as `<store>.xfs.formatting` and renamed to
   `<store>.xfs.partial` once formatted, then mounted beside the store.
4. Each base, file and directory is copied in with its owners, modes, times
   and extended attributes, synced, and its original removed.
5. Each Cloud's latest save is copied in; its previous save is cloned from
   it, the 4 KiB blocks in which the two differ written, and a block that is
   a hole or zeros in it punched.
6. The fstab line is written, the file renamed to `<store>.xfs`, and mounted
   at the store.

While the partial file exists, the store's directory holds only part of the
store, and the manager of any release with this refuses to start on it; the
update's next run goes on with the move from where it stopped, after a
restart of the machine too. A run that goes on after the machine
restarted stops the manager its unit started before it moves anything. A
failed move never returns the server to the release it left, whose manager
would start on a store half moved.

**No large write fills the disk.** Before a write that adds a large file to
the host's disk, its writer reads the host filesystem's free bytes
(`statvfs`, about a microsecond) and checks them against what the write adds
plus a reserve of 2 GiB, the room the backend's databases and logs keep
writing into meanwhile. On XFS in a file, the host's free bytes are the
check's, read under the loop device's backing file, never XFS's own, which
counts space the host may not have. The large writes are:

* a copy of a Cloud's disks that shares no extents, at a wake or a
  checkpoint, by the bytes the disks hold;
* an image's import, by its archive, then by the bytes its members take
  unpacked;
* a release's download, by its assets' sizes, then its unpacking, by its
  members';
* the databases an update keeps, whose check gains the reserve;
* a move of the store, as above.

`demi-server setup` makes the store before it checks that the machine can
run Cloud, so the check runs on the store the manager will use. A setup that
continues and finds a store holding anything leaves it, for an update to
move in while no manager runs on it.

The manager's writes run at once, one per device, so each check counts the
bytes the writes that checked before it still hold as taken, until they end;
two copies that each fit alone never fill the disk together.

When the host lacks the room for a Cloud's copy, the manager drops that
Cloud's previous save, which holds only what changed since; another user's
saves are never dropped for it. When it still lacks the room, the write is
refused, with the shortfall:

* A refused checkpoint is skipped, and the Cloud runs on its working copy.
* A refused wake leaves the Cloud stopped, and says why.
* An update's preparation fails at its step, and Settings › Updates shows it
  as it shows a failed step today.

Each drop is logged with the device, the save and the bytes it freed
(`cloud_save.dropped_for_room`), and each refusal with what it needed and
what was free (`store_room.refused`).

What an older piece meets:

* A rollback to an earlier release keeps the store where it is: its manager
  works on the mounted directory as on any other.
* The move needs no new step and no new journal entry, so an older
  `demi-server`, a page loaded before the update and an older backend read
  every record of it.

## Spike

Measured on that server, against a sparse XFS file with `reflink=1` on its
ext4 disk, with one real save of 1.7 GB, then removed:

| What | ext4, copied | XFS in a file, cloned |
|---|---|---|
| A wake's working copy | 4.1 s, 1.7 GB | 0.07 s, 0 B |
| A checkpoint's save | a full copy | 0.12 s, 0 B |
| 300 MB the Cloud writes | — | 0.29 GB more |
| Sequential 512 MB direct write | 537 MB/s | 363 MB/s |
| Sequential 512 MB read | 680 MB/s | 785 to 976 MB/s |
| 32,768 scattered 4 KiB overwrites | 1.3 to 1.6 s | 1.2 to 1.5 s |
| The same, on data shared with a save | — | 3.5 to 3.8 s |

The cost is the first write to a block a save shares, which copies it:
scattered small writes after a save run about 2.4 times slower until each
block is written once, and fragment the file (39 extents became 50,000).
Writing it again ran at 1.2 s in one run and 2.9 s in another. Neither
mitigation helped: a `cowextsize` hint of 1 MiB made the first writes
slower, and `xfs_fsr` removed the fragments by unsharing the file, which
undoes the saving. So the store keeps XFS's defaults and is not
defragmented.

## Prior art

* Incus makes a storage pool without a disk of its own as a sparse file of
  30 GiB, or a fifth of the free space when less is free, attached with
  `losetup --direct-io=on` and formatted btrfs or ZFS
  ([`loopFileSizeDefault`, `loopDeviceSetup`](https://github.com/lxc/incus/blob/main/internal/server/storage/drivers/utils.go)).
  For a VM's disk file on btrfs it turns copy-on-write off (`chattr +C`),
  since it fragments such files.
* Kubernetes' kubelet removes unused images once disk use passes
  `imageGCHighThresholdPercent`, and evicts pods under `nodefs.available`
  ([Garbage collection](https://kubernetes.io/docs/concepts/architecture/garbage-collection/#containers-images)).

Demi departs from Incus in the filesystem, XFS, whose clones a disk file
uses without turning sharing off and which needs no balancing, and in the
file's size, the host's, since the room check guards the host. It departs
from the kubelet in checking before each large write, by what that write
needs, rather than by a usage percentage swept on a schedule.

## Alternatives

* **Keep no previous save:** every Cloud loses its fallback, to spare what
  sharing makes nearly free.
* **Skip a save that changed nothing, on ext4:** a boot writes the
  filesystem's superblock, so a save that changed nothing is rare to tell
  apart, and every save that changed a little is still a full copy.
* **btrfs in the file, as Incus:** a disk file needs copy-on-write turned off
  there, and btrfs needs balancing when its metadata fills.
* **LVM thin volumes:** a Cloud's disks would be block devices, not files,
  which the store, its links and its moves are built on.

## Open questions

None.
