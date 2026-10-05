# Upgrades

An upgrade moves a server from one release of Demi to another, and with it
everything that release serves: the web app, every paired device's runner,
every Cloud and the stored data. One command does it, and a failed upgrade
leaves the server on the release it ran before.

For example, a server runs 0.1.0 and 0.2.0 is published. Its operator runs:

```sh
sudo demi-server upgrade
```

`demi-server` downloads 0.2.0, checks it, unpacks it beside 0.1.0 and checks
the configuration against it, all while 0.1.0 keeps serving. Then it stops the
backend and the machine manager, copies the databases that 0.2.0 will
migrate, points the server at 0.2.0 and starts both services again. The
interruption lasts as long as a restart, plus the copy. Afterwards:

- Open pages show "Demi Is Restarting" while the backend is away, then load
  the new web app by themselves
  ([A page of another build](../product/web-application.md#a-page-of-another-build)).
- A turn that was running ends with the shutdown's error record, and its
  conversation offers **Resume**
  ([Recovering an unfinished turn](../product/product.md#recovering-an-unfinished-turn)).
- Each paired device's runner reconnects, finds that the backend serves
  another runner release, replaces itself with it and connects again, without
  its user doing anything ([Runner updates](../execution/runner.md#runner-updates)).
- Each Cloud, hibernated by the shutdown, wakes with 0.2.0's runner and
  command programs. Its system, the packages the user installed, stays on the
  image it was created or last reset with, until the user resets it
  ([Demi's programs in a Cloud](../cloud/managed-hosts.md#demis-programs-in-a-cloud)).
- The control database is migrated as the backend starts, and each
  conversation's database when the conversation is next opened
  ([Schemas and migrations](../backend/storage.md#schemas-and-migrations)).

If anything fails before 0.2.0 reports ready, `demi-server` puts 0.1.0 back,
with the databases as they were, and says what failed. Once 0.2.0 runs,
`demi-server rollback` returns to 0.1.0 in the same way.

## One release on a server

A server runs one release at a time, and every part of Demi on it, and every
Host it serves, follows that release. Nothing negotiates versions: the
backend, the machine manager, the web app, the runners and the command
programs of a deployment are always built from one workspace version, so
none of the wires between them needs to accept another version. The few
things that do cross from one release to the next are listed in
[What crosses releases](#what-crosses-releases), and each has a rule.

A server keeps its releases under `/opt/demi`, and the services run whichever
one `current` points to:

```text
/opt/demi/
  releases/0.1.0/            a server release root, unpacked, never changed
  releases/0.2.0/            (builds-and-releases.md § Server release)
  current -> releases/0.2.0  the release the services run
/etc/demi/demi.env           the configuration, shared by both services
/etc/systemd/system/
  demi-backend.service       copied from the current release's systemd/
  demi-machine-manager.service
<DEMI_BACKEND_DATA>/         the backend's data directory, with snapshots/
<DEMI_MANAGED_DATA>/         the machine manager's state directory
/var/lib/demi/server/        demi-server's lock and the journal of an upgrade
```

- **Releases.** Each root is the [server release](builds-and-releases.md#server-release)
  of the server's Linux target, unpacked from its two archives. A root is
  never changed once unpacked; a new release is a new directory. A server
  keeps the current release and at most one other, the one a rollback
  returns to.
- **`current`.** A symbolic link, replaced atomically by renaming a new link
  over it. The units run `/opt/demi/current/bin/demi-backend` and
  `/opt/demi/current/bin/demi-machine-manager`. Each program takes its
  release root from the real path of its own executable
  ([Configuration](../backend/backend.md#configuration)), so both services
  always run the same release, and the configuration file does not set
  `DEMI_RELEASE`.
- **Units.** Each release carries its units in `systemd/`, and
  `demi-server` copies them into place when it switches releases, so a
  release that changes how a service runs brings that change itself. The
  backend runs as the system user `demi`, a member of the `demi-cloud`
  group that may use the manager's socket
  ([Storage and service setup](../cloud/setup.md#storage-and-service-setup)).
  Both units use `Type=notify` with no start timeout: a service reports
  ready only once it serves, and a long migration or image import is not a
  failure.
- **Configuration and data.** `/etc/demi/demi.env` and the two data
  directories belong to the installation, not to a release; no upgrade
  rewrites them. Moving to a release never edits the configuration: a
  release that needs a new setting fails the [check](#prepare) and names it,
  and the operator adds it.

## What follows the release

| Part | How it reaches the new release | When |
| --- | --- | --- |
| Backend and web app | The backend restarts from `current`; an open page shows the restart screen meanwhile and then loads the new build ([A page of another build](../product/web-application.md#a-page-of-another-build)) | During the upgrade |
| Machine manager and `runsc` | The manager restarts from `current`; the release root carries the pinned `runsc` of its architecture in `runtime/` ([Server release](builds-and-releases.md#server-release)) | During the upgrade |
| Cloud image | The manager imports the release's `image/` before the services stop ([Prepare](#prepare)) | During the upgrade |
| A Cloud's runner and command programs | The manager mounts the configured image's `/opt/demi` into every Cloud at boot ([Demi's programs in a Cloud](../cloud/managed-hosts.md#demis-programs-in-a-cloud)) | The Cloud's next wake |
| A Cloud's system | Stays on the image it is pinned to; Cloud settings says when a newer one is available, and a reset moves to it ([Cloud settings](../product/product.md#cloud-settings)) | When the user resets |
| A paired device's runner | The runner updates itself when the backend names another runner release ([Runner updates](../execution/runner.md#runner-updates)) | Its next connection |
| A paired device's command programs | The backend's catalog names the new packages; the runner downloads each the first time it needs it ([Install artifacts](../execution/native-runtime.md#install-artifacts)) | Each program's next use |
| Control database | Migrated as the backend starts ([Schemas and migrations](../backend/storage.md#schemas-and-migrations)) | During the upgrade |
| Conversation databases | Each migrated when its conversation is next opened | On use |
| Machine manager state | Migrated as the manager starts, when a release changes its format ([Startup and recovery](../cloud/managed-hosts.md#startup-and-recovery)) | During the upgrade |
| Object store | Nothing: objects are immutable, and a release never rewrites one or changes what a key means | Never |

A Cloud's system stays where it is because a newer image cannot go under it.
The system image holds the user's changes as an overlay of the base: the
packages the user installed are files in it, and so is the package
database, whose copy would hide the new base's own. Putting a new base under
that layer would leave a system whose package database matches neither.
Demi's own programs do not have that problem, since nothing a user installs
changes `/opt/demi`, so they follow the release at every wake.

## What crosses releases

Since every part of a deployment runs one release, only these things meet a
part of another release, and each is a contract that a release never breaks:

- **The release's assets.** `demi-server` of one release downloads the next
  one by the asset names and the `SHA256SUMS` file of
  [Release workflow](builds-and-releases.md#release-workflow). Their names and
  that file's format do not change.
- **A runner's update.** A paired device's runner of any earlier release must
  understand that the backend serves another one, and how to fetch it. The
  release check before the runner's socket opens, its headers and the body
  of its 409 answer, never change; fields are only added
  ([Runner updates](../execution/runner.md#runner-updates)). Everything after
  that check is the release's own wire.
- **Stored data.** The databases, the manager's records and the object
  store's keys outlive every release. A release reads what every earlier
  release wrote, through its migrations
  ([Schemas and migrations](../backend/storage.md#schemas-and-migrations)).
- **A pinned Cloud base.** The manager boots a Cloud on the base it is
  pinned to, which an earlier release built. What the manager needs of a
  base, its users, directories and init, is part of the base's
  `formatVersion` ([Demi's programs in a Cloud](../cloud/managed-hosts.md#demis-programs-in-a-cloud)).
- **The configuration file.** It is the installation's. Each program
  refuses a setting it does not read, so a renamed or removed setting is
  reported, never silently ignored
  ([Configuration](../backend/backend.md#configuration)).

These rules hold from the first formal release on. A server upgrades from
any formal release to any later one directly; it never has to pass through
the releases in between, since each release migrates from every earlier
format.

## The upgrade

`demi-server upgrade` without a version moves to the newest published
release; `demi-server upgrade 0.2.0` names one. `--from <directory>` takes
the assets from a directory instead of the repository's GitHub releases, as a
developer's build or a server without internet access provides them. An
older version than the current one is refused: going back is a
[rollback](#rollback). An upgrade to the current version does nothing, unless
an earlier upgrade was interrupted ([Interruptions](#interruptions)).

Only one `demi-server` runs at a time: it holds a lock in
`/var/lib/demi/server/` and refuses to start beside another.

### Fetch

The installed `demi-server` downloads the server archive and the image
archive of the server's architecture and the release's `SHA256SUMS`, checks
each archive against its sum, and unpacks both into a staging directory
under `releases/`, which it renames to the version once complete. A staging
directory left by an interrupted fetch is removed. An archive entry that is
not a regular file, directory, symbolic link or hard link, or whose path or
link target is absolute or leaves the root, fails the fetch, as an image
import does ([Import and publication](../cloud/images.md#import-and-publication)).
A release already unpacked under that version is used as it is.

Then the installed `demi-server` hands the upgrade to the new release's own
`demi-server`, which carries out the rest. Each release thus moves to
itself with its own knowledge: its units, its programs' checks, its schema
versions. Only the fetch is done by an older release, which is why the
assets are a contract ([What crosses releases](#what-crosses-releases)).

### Prepare

Everything that can fail without stopping anything happens now, while the
current release serves:

1. **Configuration.** The new backend and the new machine manager each check
   `/etc/demi/demi.env` with `--check-config`, which validates every setting
   as a start would and exits. A failure names the variable.
2. **Cloud image.** The new manager imports the release's `image/` with
   `demi-machine-manager --import`, beside the running manager. Bases are
   immutable directories named by their `baseVersion` and published
   atomically, and the running manager never reads one it was not configured
   with, so the two do not meet. The import is the slow step of a manager's
   start, and doing it now keeps it out of the interruption.
3. **Data.** `demi-server` reads the schema version of each database in the
   data directory that the configuration names and compares it with the new backend's; it knows them,
   being built from the same workspace. The databases whose versions differ
   will be migrated, so they will be copied, and the file system must have
   room for the copy.
4. **Manager state.** It reads the format of the manager's state directory.
   A release that changes the format is marked as one a rollback cannot
   cross ([Rollback](#rollback)), and `demi-server` says so before it goes
   on.

### Switch

From here the server is interrupted, and `demi-server` records each step in
its journal, `/var/lib/demi/server/upgrade.json`, before taking it:

1. Stop the backend. Its shutdown ends every turn with the shutdown record,
   hibernates every Cloud and closes every runner connection
   ([Startup and shutdown](../backend/backend.md#startup-and-shutdown)).
2. Stop the machine manager. Its stop saves every device that is still
   running.
3. Copy the databases that will migrate into
   `<DEMI_BACKEND_DATA>/snapshots/<old version>/`, cloning where the file
   system can, and sync the copy. With both services stopped, the files are
   consistent.
4. Point `current` at the new release and copy its units into place.
5. Start the manager, then the backend, and wait for each to report ready.
   The backend migrates the control database before it reports ready.
6. Remove the journal, then every release and snapshot other than the new
   release and the one it replaced.

If step 5 fails, `demi-server` [returns](#rollback) to the old release at
once, prints the last lines of the failing service's log, and exits with a
failure. Nothing waits for running turns to end before step 1: an upgrade is
started when its operator chooses, and an interrupted turn is resumed by its
user.

### Interruptions

The journal lets an upgrade killed at any point be finished. The next
`demi-server` command finds it, hands it to the `demi-server` of the release
the move goes to, and that one takes the recorded step again and the steps
after it: every step can be taken twice, since a service stops or starts
twice harmlessly, the copy of the databases is made anew from databases no
release has opened since, and `current` and the units are written whole. A
release whose services do not start then returns the server as step 5 does.
The steps themselves leave nothing half done: `current` is replaced by one
rename, a migration commits in one transaction
([Schemas and migrations](../backend/storage.md#schemas-and-migrations)), and
a server that loses power after step 4 boots into the new release, whose
services migrate and start as they would have.

## Rollback

`demi-server rollback` returns to the release that the last upgrade replaced,
the other directory under `releases/`; it refuses when that release is the
newer one, as after a rollback, since going forward is an upgrade. The
earlier release's own `demi-server` carries out the return, as each release
carries out the move to itself: it stops both services, restores the
databases from that release's snapshot when the upgrade took one, points
`current` back, copies the old units, and starts both services. A return
whose services do not start leaves its journal, and the next command tries
it again.

Restoring a snapshot loses what the backend wrote since the upgrade, such as
the conversations of that time. `demi-server` says how long ago the snapshot
was taken and asks before it restores one; the databases it replaces move to
`snapshots/abandoned-<new version>/`, so nothing is deleted. An upgrade that
migrated nothing took no snapshot, and its rollback loses nothing.

Two things a rollback leaves as they are:

- **Cloud disks.** They are not part of the snapshot. A Cloud keeps what it
  wrote after the upgrade; its runner and programs return to the old
  release at its next wake, since they come from the configured image.
- **The manager's state format.** A release that changed it cannot be rolled
  back, since its disks and records are in the new format; `demi-server`
  refuses and names that release. Such a release says so before it is
  installed ([Prepare](#prepare)).

## `demi-server`

`demi-server` is a Linux executable of the server release, built for the two
Linux targets with the backend and the manager
([Executables and targets](builds-and-releases.md#executables-and-targets)),
and the server's program for its own installation: it owns the layout above,
fetching, preparing, switching and rolling back, and the units.
`demi-server status` prints the current release, the one a rollback returns
to and whether its rollback restores a snapshot, and an interrupted upgrade
if there is one.

It downloads through the artifact library, as every other download of Demi's
does ([`shared-artifacts`](../architecture/crates-and-packages.md#shared-artifacts)).
It changes nothing outside the layout above: it never edits the
configuration, never touches a data directory except for its `snapshots/`
and a restore, and asks systemd to start and stop only Demi's two services.
Installing a server, the first time, is the installer's.

## Acceptance

Tests protect each mechanism where it is observable, without a real server:
a database of an earlier schema migrates and one of a newer schema is
refused; a runner answered with 409 replaces itself and connects with the
new release, and one whose download does not match stays on its release; the
backend answers the release check; `demi-server`, with its services
simulated, returns to the old release when a service does not start and
finishes or undoes an upgrade interrupted after any step; a page shows the
restart screen and loads a new build once.

One real upgrade accepts the whole: a Linux server with a paired device and
a Cloud with a package its user installed moves from one release to the
next. Afterwards the page runs the new build, the device is online with the
new runner without anyone touching it, and the Cloud runs the new runner and
still has the package.
