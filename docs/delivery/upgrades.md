# Upgrades

An upgrade moves a server from one release of Demi to another, and with it
everything that release serves: the web app, every paired device's runner,
every Cloud and the stored data. One command does it, and a failed upgrade
leaves the server on the release it ran before.

For example, a server runs 0.1.0 and 0.2.0 is published. Its operator runs
the following command, or a master or an administrator clicks **Update Now**
in Settings › Updates ([From Settings](#from-settings)):

```sh
sudo demi-server upgrade
```

`demi-server` takes these steps:

1. Downloads 0.2.0, checks it, unpacks it beside 0.1.0 and checks the
   configuration against it, all while 0.1.0 keeps serving.
2. Stops the backend and the machine manager.
3. Copies the databases that 0.2.0 migrates.
4. Points the server at 0.2.0 and starts both services again.

The interruption lasts as long as a restart, plus the copy. After the upgrade,
the following happens:

- Open pages show the banner "Demi is restarting" over what they show while
  the backend is away, then load the new web app by themselves
  (`packages/web-ui/src/infra/build-reload.ts`).
- A turn that was running ends with the shutdown's error record and goes on by
  itself when the new backend runs. When the user turned that off, its
  conversation offers **Resume** instead
  (`packages/web-ui/src/agent/dock-cards.ts`).
- Commands running on Hosts end:
  - A paired device's runner replaces itself, and a Cloud restarts. The shell
    work of a command runs in its runner.
  - The report of each node that ran a command says that the upgrade lost the
    command. A command the node might be waiting for gets the report at once.
    A service gets it at the node's next turn
    (`crates/agent-session/src/reports.rs`).
  - A backend that only restarts, on the same release, ends no command
    (`crates/runner-client/src/device_jobs.rs`).
- Each paired device's runner reconnects and finds that the backend serves
  another runner release. It replaces itself with that release and connects
  again, without its user doing anything (`crates/runner/src/update.rs`).
- The new backend restarts each Cloud that the shutdown left running, with
  0.2.0's runner and command programs. A Cloud's system, which holds the
  packages the user installed, stays on the image the Cloud was created or last
  reset with, until the user resets it
  (`crates/machine-manager/src/sandbox/mod.rs`).
- The backend migrates the control database as it starts. It migrates each
  conversation's database when the conversation is next opened
  (`crates/backend-database/src/lib.rs`).

If anything fails before 0.2.0 reports ready, `demi-server` puts 0.1.0 back,
with the databases as they were, and says what failed. After 0.2.0 runs,
`demi-server rollback` returns to 0.1.0 in the same way.

## One release on a server

A server runs one release at a time, and every part of Demi on it, and every
Host it serves, follows that release.

Nothing negotiates versions. The backend, the machine manager, the web app,
the runners and the command programs of a deployment are always built from one
build version, so none of the wires between them needs to accept another
version. The few things that do cross from one release to the next are listed
in [What crosses releases](#what-crosses-releases), and each has a rule.

Everything of Demi's on a server lies in one directory, `/opt/demi`, so an
operator finds all of it in one place. The services run whichever release
`current` points to:

```text
/opt/demi/
  releases/0.1.0/            a server release root, unpacked, never changed
  releases/0.2.0/
  current -> releases/0.2.0  the release the services run
  gvisor/<runsc version>/    the gVisor runtime a release's manager pins
  config/demi.env            the configuration, shared by both services
  data/backend/              the backend's data directory, with snapshots/
  data/cloud/                the machine manager's state directory
  server/                    demi-server's lock, an upgrade's journal and a setup's marker
```

Only what the system requires elsewhere lives outside `/opt/demi`:

- The two units in `/etc/systemd/system/`.
- The link `/usr/local/bin/demi-server` to
  `/opt/demi/current/bin/demi-server`.
- The system user `demi` with the group `demi-cloud`.

`data/cloud` can be another directory, such as one on a dedicated filesystem
([Storage and service setup](../cloud/setup.md#storage-and-service-setup)).
The configuration names it.

The parts of the layout follow these rules:

- **Releases.** Each root is the server release
  (`crates/xtask/src/server_release.rs`) of the server's Linux target,
  unpacked from its two archives. `demi-server` never changes a root after it
  unpacks it: a new release is a new directory. A server keeps the current
  release, the one a rollback returns to, and at most one ready ahead of an
  update ([Ready ahead](#ready-ahead)).
- **gVisor.** No release carries gVisor. Each machine manager is built against
  one pinned version of `runsc`, and `demi-server` fetches that version into
  `gvisor/<version>/` before a release that needs it runs
  (`crates/server/src/gvisor.rs`). Versions sit beside each other, so a
  rollback finds its own. A move keeps the versions of its two releases and
  removes the others.
- **`current`.** A symbolic link, replaced atomically by renaming a new link
  over it. The units run `/opt/demi/current/bin/demi-backend` and
  `/opt/demi/current/bin/demi-machine-manager`. Each program takes its release
  root from the real path of its own executable
  (`crates/backend/src/config/mod.rs`), so both services always run the same
  release, and the configuration file does not set `DEMI_RELEASE`.
- **Units.** Each release carries its units in `systemd/`, and `demi-server`
  copies them into place when it switches releases, so a release that changes
  how a service runs brings that change itself. The backend runs as the system
  user `demi`, a member of the `demi-cloud` group that can use the manager's
  socket ([Storage and service setup](../cloud/setup.md#storage-and-service-setup)).
  Both units use `Type=notify` with no start timeout: a service reports ready
  only when it serves, and a long migration or image import is not a failure.
- **Configuration and data.** `config/demi.env` and the two data directories
  belong to the installation, not to a release, and no upgrade rewrites them.
  Moving to a release never edits the configuration. A release that needs a
  new setting fails the check in [Prepare](#prepare) and names the setting,
  and the operator adds it.

## What follows the release

The following table shows how each part reaches the new release, and when:

| Part | How it reaches the new release | When |
| --- | --- | --- |
| Backend and web app | The backend restarts from `current`. An open page shows a spinner over its conversation's end, above the composer, meanwhile and then loads the new build (`packages/web-ui/src/infra/build-reload.ts`) | During the upgrade |
| Machine manager and `runsc` | The manager restarts from `current`, with the `runsc` version it pins, which the preparation fetched ([Prepare](#prepare)) | During the upgrade |
| Cloud image | The manager imports the release's `image/` before the services stop ([Prepare](#prepare)) | During the upgrade |
| A Cloud's runner and command programs | The manager mounts the configured image's `/opt/demi` into every Cloud at boot (`crates/machine-manager/src/sandbox/mod.rs`) | The Cloud's next wake |
| A Cloud's system | Stays on the image it is pinned to. Cloud settings says when a newer one is available, and a reset moves to it (`packages/web-ui/src/cloud/CloudSettings.vue`) | When the user resets |
| A paired device's runner | The runner updates itself when the backend names another runner release (`crates/runner/src/update.rs`) | Its next connection |
| A paired device's command programs | The backend's catalog names the new packages, and the runner downloads each the first time it needs it (`crates/runner-command-packages/src/cache.rs`) | Each program's next use |
| Control database | Migrated as the backend starts (`crates/backend-database/src/lib.rs`) | During the upgrade |
| Conversation databases | Each migrated when its conversation is next opened | On use |
| Machine manager state | Migrated as the manager starts, when a release changes its format (`crates/machine-manager/src/main.rs`) | During the upgrade |
| Object store | Nothing: objects are immutable, and a release never rewrites one or changes what a key means | Never |

A Cloud's system stays on its image because the system image holds the user's
changes, the package database among them, as an overlay of the base
(`crates/machine-manager/src/sandbox/mod.rs` says why). Demi's own programs
follow the release at every wake.

## What crosses releases

Every part of a deployment runs one release, so only a few things meet a part
of another release. Each is a contract that a release never breaks:

- The release's assets and their names.
- A runner's update check.
- The stored data.
- A pinned Cloud base.
- A stored thing that a release moves to its new place.
- The configuration file.

A server upgrades from any release in the history to any later one directly.
`crates/server/src/main.rs` lists the contracts. The history starts with
0.1.11, the first release whose databases migrate
(`crates/backend-database/src/lib.rs`).

## The upgrade

`demi-server upgrade` takes the release to move to in one of four ways:

- With `--beta`, it moves to the newest release Demi published.
- With `--canary`, it moves to the newest release, the Canary pre-releases
  included.
- With a version, as in `demi-server upgrade 0.2.0`, it moves to that one.
- With none of them, it moves to the newest release of the server's update
  channel, the one Settings › Updates shows: the channel an administrator
  chose there, or else the one the server was installed from,
  `DEMI_UPGRADE_CHANNEL` in the configuration, Beta when that names none.

The newest release is the highest version among the repository's releases
tagged `v<version>`. `releases/latest` gives it for published releases, and
the list of releases gives it for all. A release of another tag, such as the
gVisor build's, is no Demi release.

`--from <directory>` takes the assets from a directory instead of the
repository's GitHub releases, as a developer's build or a server without
internet access provides them.

Two cases do not move the server:

- `demi-server` refuses an older version than the current one: going back is a
  [rollback](#rollback).
- An upgrade to the current version does nothing, unless an earlier upgrade
  was interrupted ([Interruptions](#interruptions)).

Only one `demi-server` runs at a time: it holds a lock in `/opt/demi/server/`
and refuses to start beside another.

### Fetch

The installed `demi-server` takes these steps:

1. Downloads the server archive and the image archive of the server's
   architecture, and the release's `SHA256SUMS`.
2. Checks each archive against its sum.
3. Unpacks both into a staging directory under `releases/`, which it renames
   to the version when the unpacking is complete.

The fetch follows these rules:

- It removes a staging directory that an interrupted fetch left.
- An archive entry fails the fetch when it is not a regular file, directory,
  symbolic link or hard link, or when its path or link target is absolute or
  leaves the root, as in an image import
  (`crates/machine-manager/src/storage/base.rs`).
- It uses a release that is already unpacked under that version as it is.

Then the installed `demi-server` hands the upgrade to the new release's own
`demi-server`, which carries out the rest. Each release thus moves to itself
with its own knowledge: its units, its programs' checks, its schema versions.

Only the fetch is done by an older release, which is why the assets are a
contract ([What crosses releases](#what-crosses-releases)).

### Prepare

Everything that can fail without stopping anything happens now, while the
current release serves. Settings › Updates lists these steps, with their
progress where they have any ([From Settings](#from-settings)), all but the
runners', which no step waits for:

1. **Runners.** As the preparation starts, `demi-server` tells the running
   backend the next runner release and the new release's directory that holds
   its executables.
   - It does so over the backend's management socket,
     `/run/demi-backend-manage/manage.sock`, which only the backend's user and
     root can open.
   - The socket's directory is the backend unit's second runtime directory,
     apart from `/run/demi-backend/`, which every Cloud mounts
     (`crates/backend-http/src/lib.rs`). A Cloud's root must never reach the
     socket, because it takes requests without credentials and acts on every
     user's devices.
   - The backend has its connected paired runners download the release ahead
     (`crates/runner/src/update.rs`) and answers as they finish.
   - `demi-server` prints how many have, *3 of 4 devices have the new runner*,
     while the next steps run, and waits for none of them. A device that has
     not finished by the restart downloads after it, and shows as updating
     until its runner is back.
   - A backend of a release without the socket is told nothing, and its
     runners update after the switch as before.
2. **Configuration.** The new backend and the new machine manager each check
   `/opt/demi/config/demi.env` with `--check-config`, which validates every
   setting as a start would and exits. A failure names the variable.
3. **gVisor.** `demi-server` fetches the `runsc` version that the new manager
   pins into `/opt/demi/gvisor/<version>/`, unless it is there
   (`crates/server/src/gvisor.rs`).
4. **Cloud image.** The new manager imports the release's `image/` with
   `demi-machine-manager --import`, beside the running manager. Bases are
   immutable directories named by their `baseVersion` and published
   atomically, and the running manager never reads one it was not configured
   with, so the two do not meet. The import is the slow step of a manager's
   start, and doing it now keeps it out of the interruption.
5. **Data.** `demi-server` reads the schema version of each database in the
   data directory that the configuration names, and compares it with the new
   backend's. It knows them, because it is built from the same workspace.
   - The databases whose versions differ are migrated, so they are copied, and
     the file system must have room for the copy.
   - A database whose version is neither the new schema's nor in its history
     stops the upgrade here, naming the file, before anything is interrupted:
     the new backend could not open it. A development build of another
     unpublished schema can make such a database.
6. **Manager state.** `demi-server` reads the format of the manager's state
   directory. A release that changes the format is marked as one a rollback
   cannot cross ([Rollback](#rollback)), and `demi-server` says so before it
   goes on.

### Switch

From here the server is interrupted. `demi-server` records each step in its
journal, `/opt/demi/server/upgrade.json`, before it takes the step:

1. Stop the backend. Its shutdown ends every turn with the shutdown record
   and closes every runner connection
   (`crates/backend/src/backend.rs`).
2. Stop the machine manager. Its stop saves and stops every device that is
   still running, so each Cloud boots with the new release's programs. The
   manager writes how many of the running Clouds it has saved to
   `/run/demi-machine-manager/drain.json`, which `demi-server` reads while it
   waits for the stop.
3. Keep the databases that the new release changes in
   `<DEMI_BACKEND_DATA>/snapshots/<old version>/`, and sync them: a copy of
   each, cloned where the file system can. With both services stopped, the
   files are consistent.
4. Point `current` at the new release and copy its units into place.
5. Start the manager, then the backend, and wait for each to report ready.
   The backend listens at once, migrates the control database and every
   conversation database, and only then reports ready
   (`crates/backend-http/src/update.rs`). Until then it answers every page
   with the update's progress, which each page shows over the app, and
   `demi-server` prints it, `Updating conversations: 41 of 73 (2.7 GB of
   6.6 GB), about 3 minutes left`.
6. Remove the journal, then every release and snapshot other than the new
   release and the one it replaced.

From step 1 until the new backend listens, no backend answers the pages. The
new release's backend then runs in its update screen's mode,
`demi-backend --update-screen <record>`, as the transient unit
`demi-update-screen.service` with the backend unit's user, capability and
configuration. It opens no database: it serves the web app and answers
`GET /api/update` with the update's steps, which `demi-server` keeps in
`<DEMI_BACKEND_DATA>/update.json` as it takes them. `demi-server` stops the
screen just before it starts the backend, which reads the same file, goes on
with its own steps and removes the file once it serves. A return shows the
screen again while it runs.

If step 5 fails, `demi-server` returns to the old release at once (see
[Rollback](#rollback)), prints the last lines of the failing service's log,
and exits with a failure.

Nothing waits for running turns to end before step 1. An operator starts an
upgrade when they choose, and the user resumes an interrupted turn.

### Interruptions

The journal lets `demi-server` finish an upgrade that was terminated at any
point. The next `demi-server` command finds the journal and hands the upgrade
to the `demi-server` of the release the move goes to. That one takes the
recorded step again and the steps after it.

- A step after step 1 that fails leaves the update screen running, with
  the failed step, until the next command finishes the move; that command
  stops the screen before it starts the backend.
- A release whose services do not start returns the server as step 5 does.
- The next command finishes a return that is terminated or does not start
  either.
- Either way, the release the command reports is the one `current` names.

The steps themselves leave nothing half done:

- One rename replaces `current`.
- A migration commits in one transaction
  (`crates/backend-database/src/lib.rs`).
- A server that loses power after step 4 boots into the new release, whose
  services migrate and start as they would have.

`crates/server/src/journal.rs` holds the reasons.

## From Settings

A master or an administrator updates the server from Settings › Updates,
without a shell on the machine. The page shows the release the server runs
and the newest release of its channel:

- **Beta** offers the releases Demi publishes, which GitHub marks *latest*.
- **Canary** offers every release, including the pre-release that each push
  to `main` publishes once it passes its checks.

The backend looks for the newest release when it starts, when the channel
changes, on **Check Now**, and then every hour on Canary and every day on
Beta. While a newer one exists, the sidebar shows a green update icon to the
master and administrators.

### Ready ahead

After every check that finds a newer release, the server gets it ready while
the current one serves, so **Update Now** has only the checks and the restart
left:

1. The backend writes the version to `/run/demi-upgrade/prepare`.
2. The path unit `demi-prepare.path` sees the file and starts
   `demi-prepare.service`, which runs `demi-server prepare --requested` as
   root.
3. `demi-server` removes the request and does nothing while another run of it
   holds its lock, or while a move is interrupted. Otherwise it removes a
   release that was ready before this one, fetches the release as
   [Fetch](#fetch) does, and hands over to the release's own `demi-server
   prepare`. That one downloads its gVisor version, imports its image, and
   tells the running backend its runner release, which the connected devices
   download ([Prepare](#prepare)).
4. It tells the backend each step, which Settings › Updates shows on the
   release's row: *Downloading in the background*, *Installing in the
   background…*, then *Ready to install*, or why it could not. The next check
   asks again, so a backend started since hears of the release by then, and
   so do the devices that were away.

An update that starts while this runs waits for it, then finds its steps done
and shows them as already installed.

One release is ready at most. The server keeps its current release, the one a
rollback returns to, and the one ready ahead, with their gVisor versions. The
manager's import beside a running manager records the base it imported in
`ahead.json` in its image store and removes the one recorded before it, which
no manager ran and no Cloud is pinned to. A device keeps the runner and the
programs of the release it runs and of the one ready ahead; its download of a
newer one removes the earlier one's.

**Update Now** asks first, then runs the same upgrade as
`sudo demi-server upgrade <version>`:

1. The backend writes the version to `/run/demi-upgrade/version`. It runs as
   an unprivileged user, so it cannot upgrade the server itself.
2. The path unit `demi-upgrade.path` sees the file and starts
   `demi-upgrade.service`. That unit runs
   `demi-server upgrade --requested` as root, apart from the backend that the
   upgrade stops.
3. `demi-server` removes the request, refuses a version that is not newer
   than the current one, and upgrades as above.
4. While it runs, `demi-server` tells the backend every step of the update
   over the backend's management socket, each with its state and its
   progress: the bytes downloaded, the Clouds saved, the databases copied.
   The administrators' pages list the steps of [Fetch](#fetch) and
   [Prepare](#prepare) in Settings › Updates. From the stop on, every page
   shows the update's screen, which lists the steps of [Switch](#switch) and
   the new backend's own. Once the new release serves, the screen shows every
   step done and waits for the user to open Demi. A step that fails is marked
   as failed with its reason.

The units come with the release, as the services' units do. A server on an
earlier release gets them with one `sudo demi-server upgrade` to a release
that has them. Without them,
or on a developer's machine, Settings has no Updates.

`journalctl -u demi-upgrade` shows what an update from Settings did.

## A development build on a server

For example, a batch of work is merged and the user wants to try it on their
own server before it reaches Beta. The server moves to the next version,
built on GitHub from the branch, as an upgrade moves it to a
published release. Pages reload into the new web app, the user's paired
devices replace their runners, and Clouds wake with the new programs, exactly
as after an upgrade. Nothing is compiled on the developer's machine.

Every commit of the branch has a version of its own: `0.1.` and the number of
commits in its history, `0.1.4213` for a commit with 4,213 commits behind it.
The same commit always has the same version, and a later commit a higher one.
Nothing is committed to deploy, and versions skip numbers between releases.

Every push to `main` builds everything a release publishes, for every
target (`.github/workflows/build.yml`). Once that build and the push's checks
(`ci.yml`) have passed, `canary.yml` publishes it as a pre-release, the next
Canary. A deploy needs the pre-release of the newest pushed commit: it usually
finds it published already, and otherwise asks for it and waits. The commit
and its version are worked out first, so every later step names them, and a
version that has a release already, as a Canary or from an earlier deploy, is
not published again:

```sh
git fetch origin main
commit=$(git rev-parse FETCH_HEAD)
version=0.1.$(git rev-list --count "$commit")
published() {
  [ "$(gh release view "v$version" --json isDraft -q .isDraft 2> /dev/null)" = false ]
}
if ! published; then
  run=$(gh workflow run release.yml --ref main -f commit="$commit" -f prerelease=true)
  gh run watch --exit-status "${run##*/}" || true
  published
fi
```

`gh workflow run` prints the URL of the run it started, which `gh run watch`
takes by its number. A release makes a draft, uploads the files into it and
publishes it last, and `gh release view` finds a draft too, so a release
counts only once it is no draft. A release that `canary.yml` started for the
same commit meanwhile makes this one wait behind it and then refuse the
version, so the deploy checks for the published pre-release rather than for
the run's success. Each other command that fails stops the deploy: no server
is touched until the pre-release is published.

The release finds that commit's build and publishes its files as a GitHub
pre-release, `v$version` (`.github/workflows/release.yml`):

- A build that finished takes the release a few minutes.
- A build that is running or waiting is waited for. Of the pushes that arrive
  while a build runs, only the newest waits, so the commit of a busy hour may
  never have been built.
- A commit with no build, because a newer push took its place, because it failed
  or because its files were kept for a day and are gone or will be within two
  hours, is built by the release itself, which takes as long as a build does.

A pre-release is never the latest release, which `releases/latest` gives, so
Beta never offers one: Canary does, from the installer's `--canary`,
`demi-server upgrade --canary` and Settings › Updates, until it is published.
A run that fails touches no server. A release refuses a version that has a
release already.

Then move the server, the SSH host that `DEMI_DEPLOY_HOST` names in the
developer's `.env`. It must already run Demi
([Installation](installation.md)), and the SSH user must be able to use
`sudo`:

```sh
ssh "$DEMI_DEPLOY_HOST" \
  "sudo /usr/local/bin/demi-server upgrade $version && sudo /usr/local/bin/demi-server status"
```

A server on the Canary channel offers the pre-release in Settings › Updates
once it exists, so **Update Now** there moves it as this command does.

`demi-server upgrade` downloads the pre-release from GitHub as it downloads a
published one and carries it out ([The upgrade](#the-upgrade)): the checks,
the database copies, the switch and the return on failure are an upgrade's.
`demi-server rollback` returns to the release before it.

- **Publishing it.** To publish a deployed version the user is content with,
  turn its pre-release into the latest release on GitHub with
  `gh release edit "v$version" --prerelease=false --latest`. The files users
  install are then the ones the server ran, built once. Every version is kept,
  published or not.
- **Data.** A deploy that changes a schema migrates the server's databases as
  an upgrade does.
  - A schema changed again after a deploy shipped it, in the same unpublished
    batch, leaves the server's databases at a version the next build does not
    know. The Data check of [Prepare](#prepare) then stops the upgrade before
    anything is interrupted, and names the databases. The developer resolves
    it before deploying again. Nothing is migrated by guesswork.
  - When a server keeps data of a deployed version, its schema enters the
    history as any release's does, named by its version, with a tested
    migration, so the next deploy and the next release carry the data on.
  - Every work package that changes a schema adds the schema `origin/main`
    holds to the history, with its migration, named by the version of the
    commit on main it was taken from. Any commit of main may become a Canary,
    one whose checks passed and whose build started while later work merged
    included, so any schema main held may be on a server. A step that no server needed costs only
    that step.

## Rollback

`demi-server rollback` returns to the release that the last upgrade replaced,
the other directory under `releases/`. It refuses when that release is the
newer one, as after a rollback, because going forward is an upgrade.

The earlier release's own `demi-server` carries out the return, as each release
carries out the move to itself.

1. It stops both services.
2. It restores the databases from that release's snapshot, when the upgrade
   took one: it renames each database the snapshot replaces into
   `snapshots/abandoned-<new version>/`, and puts a copy of the snapshot's
   in its place. The snapshot stays whole, so a return taken again
   after an interrupted one puts back the same databases. A return to 0.1.65
   or earlier copies every database both ways, as those releases did.
3. It points `current` back and copies the old units.
4. It starts both services.

A return whose services do not start leaves its journal, and the next command
tries it again.

Restoring a snapshot loses what the backend wrote since the upgrade, such as
the conversations of that time. `demi-server` says when it took the snapshot
and asks before it restores one. The databases it replaces move to
`snapshots/abandoned-<new version>/`, so nothing is deleted. An upgrade that
migrated nothing took no snapshot, and its rollback loses nothing.

A rollback leaves two things as they are:

- **Cloud disks.** They are not part of the snapshot. A Cloud keeps what it
  wrote after the upgrade. Its runner and programs return to the old release
  at its next wake, because they come from the configured image.
- **The manager's state format.** A release that changed it cannot be rolled
  back, because its disks and records are in the new format. `demi-server`
  refuses and names that release. Such a release says so before it is
  installed ([Prepare](#prepare)).

## The `demi-server` program

`demi-server` is a Linux executable of the server release, built for the two
Linux targets with the backend and the manager. It is the server's program for
its own installation: it owns the layout in
[One release on a server](#one-release-on-a-server), fetching, preparing,
switching and rolling back, and the units.

`demi-server status` prints:

- The current release.
- The one a rollback returns to, and whether that rollback restores a
  snapshot.
- An interrupted upgrade, if there is one.

`demi-server` changes nothing outside that layout:

- It never edits the configuration.
- It never touches a data directory except for its `snapshots/` and a restore.
- It asks systemd to start and stop only Demi's two services, and to run the
  path unit of an update from Settings.

Setting a server up, the first time, is its `setup`
([Installation](installation.md)).
