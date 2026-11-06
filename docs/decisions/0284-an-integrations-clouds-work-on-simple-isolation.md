---
status: proposed
---

# An integration's Clouds work on Simple Isolation

## Problem

Alex sets up Demi on a container platform, whose new server runs its Clouds on
Simple Isolation (0268). Alex builds a team chat app as an integration, and its
Clouds share a folder (0156). The shared folder is an ext4 image that the
manager mounts as root, so on Simple Isolation, where a manager may have no
root and a Cloud is a directory under proot, there is no folder to give the
Clouds. A Cloud's CPU, memory and disk sizes have nothing to enforce them there
either, while the settings page goes on offering them.

## Decision

An integration works on either isolation, and what Simple Isolation can't
enforce is shown as such, as Settings › Users shows a user's Cloud Storage.

| Capability | gVisor | Simple Isolation |
| --- | --- | --- |
| Shared folder | An ext4 image, mounted once, bound in | A directory, bound in with proot |
| Its size | Enforced | None; the row is disabled, with the reason |
| CPUs, memory, disks | Enforced | None; the rows are disabled, with the reason |
| Always-on, idle window, init script | Work | Work |

```text
Clouds                     Clouds on Simple Isolation have no CPU, memory or
                           disk limits. These settings apply once the server
                           runs on gVisor.
  Idle window              [ 60          ] min
  CPUs                     [ Default ▾ ] [ 2  ] (disabled)
  Memory                   [ Default ▾ ] [ 2  ] GB (disabled)
  System disk              [ 16          ] GB (disabled)
  Home disk                [ 16          ] GB (disabled)
```

`GET /integrations/server-sizes` says so:

```rust
pub struct ServerSizes {
    // ...
    /// Whether the chosen isolation enforces a Cloud's CPUs, memory and disks.
    pub limits: bool,
}
```

The sizes an integration holds stay in its record and in `GET /v1/integration`,
and take effect if the server moves to gVisor. A request that sets one on
Simple Isolation is stored, not refused, since nothing about it is wrong.

### The folder on Simple Isolation

The directory is `<data>/shared-files/<id>`. A Cloud that names the folder
starts with `-b <directory>:<path>`, the same path as on gVisor.

A manager without root runs every Cloud as itself, so they all write the
directory. A manager with root runs each Cloud as a user of its own, whose
files the others can't change. There the directory and its files belong to one
group, 60999, in the range of Debian's that the Clouds' users come from. A Cloud
that names a folder runs with the group as its own, its user's group beside it,
so what it makes belongs to the group, and its umask is `002` so the group can
write it. A file that a program makes with an explicit mode, such as `0644`,
others can read and not change, as on any Unix group folder.

The group and the umask hold in the whole Cloud, its home included, where the
other Clouds can't reach: a Cloud's tree is open only to its own user's group.
A file such as `~/.gitconfig` is then group-writable, which Ubuntu's OpenSSH
accepts for `~/.ssh/config`.

A setgid directory, the usual way to keep a folder's files in its group, isn't
enough here: under `-0`, proot sets a new directory's mode after making it,
which clears the bit, so a subfolder would lose the group.

A path with a colon can't be bound, since proot reads `host:guest`, so a shared
folder's path can't hold one, on either isolation.

### When the master switches the isolation

A Cloud moves to the chosen isolation with everything in it (0268). Its shared
folder moves too, once, with the first Cloud that wakes on the new isolation:
to a directory from the image, or to an image from the directory, sized at the
integration's size or as large as the files need, once the server's disk has
room for the copy. The new form is made whole under another name; then the
old form is renamed aside, the new one takes its place, and the old one is
removed. So the folder never has both forms, which the Clouds of each
isolation would each take for the folder. A crash midway leaves the old form
aside, which the next Cloud's first use puts back when the new form isn't in
place, and removes when it is. A folder found in both forms anyway, such as
one an operator restored, refuses to start until someone keeps one.

While a Cloud on the old isolation still runs, the folder is in use in the old
form, and another Cloud's wake on the new isolation is refused, with a message
that says why. The turn or command that woke it fails with that message. The
alternative of each Cloud seeing its own copy would silently split the folder.

The copy runs during that wake, so a 400 GB folder holds it for as long as the
copy takes, and a manager stopping meanwhile waits for it or for systemd's
limit. A switch of isolation is a rare, deliberate act of the master's, and
Cloud moves already take minutes.

## Spike

Each point ran on a Linux VM, a manager running as root with a release build
of this change, and a backend driven through the integration API:

* **Two Clouds write each other's files.** Cloud A made a file and nested
  folders; Cloud B, another user, appended to the file, made a file in A's
  folder and removed one; A then changed B's file. With a setgid folder alone,
  a subfolder that A made under proot lost the bit, and B's file in it took
  B's own group, which A couldn't change: hence the primary group. A file
  copied with its mode kept, such as by `cp -r` of `0644` files, the other
  Cloud can read and not remove, as the decision says.
* **A path that doesn't exist in the Cloud.** With the path `/data/x`, proot
  made `/data` and bound the folder there.
* **The idle window, always-on and the init script** work on Simple
  Isolation as on gVisor: the init script ran at the first start, and two
  Clouds stopped after the 2-minute window.
* **A switch with a Cloud still running.** With A on Simple Isolation and the
  master's choice gVisor, a new Cloud's command failed with "the shared folder
  … is in use by Clouds on the other isolation until they stop". Once A
  stopped, the new Cloud woke and moved the folder.
* **Both moves keep the files.** Directory to image and back again, every
  file and folder came through, `lost+found` stayed out of the directory, and
  each Cloud wrote at the folder's root afterwards. The first try found the
  image's root left to root, which `demi` couldn't write, and fixed it.
* **A crash during a move.** With the directory renamed aside and no image in
  place, the next wake put the directory back and moved it again, every file
  kept. With the directory in place and the image aside, the next wake removed
  the image. With both forms in place, a wake failed with "needs recovery: it
  has both an image and a directory".
* **A manager restart.** With A running, the restarted manager took it over
  and kept counting it on the folder: A went on writing, and a Cloud on gVisor
  was still refused.
* **Deleting the integration** removed the directory, the image and the
  Clouds' trees.

## Prior art

proot's `-b host:guest` ([proot](https://proot-me.github.io/)) binds a host
path at a path in the guest, which is how this folder reaches a Simple
Isolation Cloud, as Docker's [bind
mounts](https://docs.docker.com/engine/storage/bind-mounts/) bring a host
directory into a container: the folder takes its space from the host's
filesystem, so it has no size of its own.

Users who share files on one Unix server usually share a group, with a setgid
directory and a umask of `002`. Demi departs only in giving the group as the
Cloud's own instead of through the setgid bit, which proot clears.

## Alternatives

* **Integrations only on gVisor, refusing a Cloud on Simple Isolation.** The
  least change, but a new server, which starts on Simple Isolation, couldn't run
  an integration until its master switched.
* **A folder that stays in its form and is bound on either isolation.** Nothing
  would need copying, but an image needs root to mount on Simple Isolation, and
  its files belong to gVisor's user, which a Simple Isolation Cloud's user can't
  write.
* **A folder that doesn't follow a switch.** It would look empty on the new
  isolation, and its files would come back only on a switch back.
* **One user for all the Clouds of an integration.** A manager ends a Cloud by
  ending its user's processes, so a stop of one Cloud would end the others.

## Open questions

None.
