---
status: proposed
---

# An administrator updates the server from Settings

## Problem

A server learns of a new release only when someone looks on GitHub, and
moves to it only when someone with `sudo` on the machine runs
`demi-server upgrade`:

* An administrator who uses Demi in the browser never hears that a fix
  exists, and one without shell access to the machine cannot install it.
* A developer who wants the server on the newest build signs in over SSH
  for every deploy (`upgrades.md` § A development build on a server).

The backend cannot do it alone. It runs as the unprivileged `demi` user,
which may neither write `/opt/demi` nor ask systemd to stop or start a
unit, and the upgrade stops it: a program it starts lives in its unit's
control group, which systemd kills with it.

## Decision

### What the user sees

**The sidebar.** When the server's channel has a release newer than the one
it runs, a master or an administrator sees a green update icon at the
bottom of the sidebar, right of the settings button, as Chrome's toolbar
shows *Update* beside its menu and Home Assistant badges its Settings. Its
tooltip says *Demi 0.1.5500 is available*. A click opens Settings ›
Updates. A user sees nothing.

**Settings › Updates**, before About in the System group, for a
master or an administrator on a server that `demi-server` installed:

```text
Updates

  Update channel                                   [ Beta        v ]
                                                     Beta
                                                     Canary

  Demi 0.1.5500 is available                    [ Update Now ]
  You have 0.1.5469. Release notes ↗
  Last checked 5 minutes ago                     [ Check Now ]
```

* With nothing newer, the first row says *Demi is up to date* and names the
  version it runs.
* **Beta** offers the releases Demi publishes, which GitHub marks *latest*
  in turn. Demi has no stable channel: every release it publishes is a beta,
  and Chrome and Edge name such a channel Beta.
  **Canary** offers every release, the pre-releases a deploy publishes
  included, by the highest version: the newest builds, which nobody has
  tried on a server yet. The name is Chrome's and Edge's for their
  earliest, least tested channel, and stays *Canary* in every language,
  as theirs does.
* A channel never offers an older version: a server on a canary that
  switches to Beta stays where it is until a Beta release passes it,
  as Coolify and Home Assistant refuse a downgrade from their pages.
  `demi-server rollback` stays the way back.
* A check that fails says why under the row, such as *Couldn't reach
  GitHub*, and Check Now tries again.

**Update Now** asks first:

```text
  Update Demi to 0.1.5500?

  Demi restarts and is unavailable to everyone for a few minutes.
  2 agents are working; they continue after the update.

                                         [ Cancel ]  [ Update ]
```

**While it runs.**

1. The release downloads while Demi keeps serving. The row shows a bar,
   *Downloading Demi 0.1.5500*, and under it the bytes, the rate and the
   time left, *120 MB of 210 MB · 12 MB/s · Less than a minute remaining*,
   as Chrome's downloads list them, and the sidebar's icon turns into a
   spinner for every administrator, which still opens the page.
2. When the download is checked, the backend says it restarts: every page,
   every user's, shows the Updating screen of 0104 with *Restarting Demi*
   and a moving bar, through the stop, the copy of the databases and the
   start.
3. The new backend migrates its data as 0104 shows, then serves, and every
   page loads itself in the new build.

**When it fails.** `demi-server` returns to the earlier release, as any
failed upgrade does. The pages load the earlier build again, and the page
says *Couldn't update to 0.1.5500. Demi went back to 0.1.5469.* with the
reason, until the next update.

### How it works

```text
page ──Update Now──► backend ──writes──► /run/demi-upgrade/version
  ▲                    ▲                        │ demi-upgrade.path
  │ state channel      │ management socket      ▼
  └────────────────────┴───────────── demi-upgrade.service (root)
                                      demi-server upgrade --requested
```

1. **Checking.** The backend asks GitHub for the newest release of its
   channel when it starts, every 24 hours, and on Check Now, with the same
   code `demi-server upgrade` uses, moved to `demi-shared-artifacts`. The
   answer is held in memory, never stored. The bound keeps the checks far
   below GitHub's 60 unauthenticated requests an hour per address, as
   Canary reads the list of releases a page of 100 at a time.
2. **The channel** is the server's, one for every account, stored in the
   control database's new table `server_settings`. An administrator
   changes it.
3. **The request.** Update Now writes the version, and nothing else, to
   `/run/demi-upgrade/version`, in a runtime directory of the backend's
   unit that the `demi` user owns. A path unit, `demi-upgrade.path`,
   starts `demi-upgrade.service`, a oneshot unit that runs as root and
   outlives the backend's stop.
4. **The run.** `demi-server upgrade --requested` reads the file without
   following a link or waiting for a writer, removes it, takes a plain file that
   holds a version and nothing else, and refuses one not newer than the current
   release. It then upgrades as `demi-server upgrade <version>` does today: the
   same download from GitHub, checksums, journal, copies, switch and return; a
   version GitHub does not publish fails at its download.
5. **The progress.** `demi-server` tells the running backend how far it is over
   the backend's management socket, which only root may open, as it tells it the
   next runner release (0084): a line for each step, downloading with its bytes
   of the whole and its rate over the last five seconds, preparing, restarting,
   and failed with its reason. The whole is the sum of the sizes its assets' own
   addresses answer to a `HEAD`, so GitHub's API and its limit on requests stay
   out of the download. It repeats the step every 10 seconds while it downloads
   and prepares, and a backend that hears nothing for a minute takes the run as
   failed; a backend still running a minute after it heard of its restart was
   never stopped, takes the run as failed too, and every page leaves the
   Updating screen. On *restarting* the backend tells every page, and answers a
   second later, so the pages hear it before the stop. The failure is held in
   memory until the next update or the backend's next start.
6. **The pages.** The backend pushes the run to every master's and
   administrator's page. On *restarting* it pushes the new update state to
   every page, and answers `GET /api/update` with it, so a page that shows
   the screen keeps it instead of reloading into the old build.
7. **Where it applies.** The backend's unit names the request's file,
   `DEMI_UPGRADE_REQUEST=/run/demi-upgrade/version`. Without it, as on a
   developer's machine or a desktop install, the section and the icon are
   absent and the routes answer 404.
8. **The first time.** A server running an earlier release has neither
   unit: one `sudo demi-server upgrade` to a release that brings them
   installs them, as every move installs its release's units, and every
   later update is a click.

The lines on the management socket are written by the release a move goes
to and read by the running backend, an earlier release's: like the
prefetch request, their fields and steps are only ever added, and a
backend ignores a line it cannot read.

```rust
/// `GET /api/server-upgrade`, for the master and administrators.
pub struct ServerUpgrade {
    pub running: Version,
    pub channel: UpgradeChannel,
    pub check: UpgradeCheck,
    pub run: UpgradeRun,
}

pub enum UpgradeChannel { Beta, Canary }

pub enum UpgradeCheck {
    Never,
    Checking,
    Checked { at: Timestamp, newer: Option<Version>, notes_url: Option<String> },
    Failed { at: Timestamp, reason: String },
}

pub enum UpgradeRun {
    None,
    Downloading { to: Version, bytes_done: u64, bytes: u64, bytes_per_second: u64 },
    Preparing { to: Version },
    Restarting { to: Version },
    Failed { to: Version, reason: String },
}

/// 0104's update state, with the moment before the stop.
pub enum UpdateState {
    None,
    Restarting,
    Updating { /* as 0104 */ },
}
```

`PUT /api/server-upgrade/channel`, `POST /api/server-upgrade/check` and
`POST /api/server-upgrade/run { version }` change it; each answers the new
`ServerUpgrade`, which the state channel also pushes to every master and
administrator. `GET /api/server-upgrade/agents` counts the agents at work,
every account's, which the question before an update names.

## Spike

None: the path unit and the oneshot service are systemd's ordinary means,
the management socket already carries an upgrade's lines, and the upgrade
itself is unchanged.

## Prior art

* **Home Assistant** checks for updates and lists them in
  Settings › Updates for administrators, with a badge on Settings in the
  sidebar that counts them
  ([`ha-sidebar.ts`](https://github.com/home-assistant/frontend/blob/dev/src/components/ha-sidebar.ts)).
  The Supervisor, a privileged container apart from Core, runs the update,
  so it outlives Core's restart, and its channel is stable, beta or dev.
  Demi's `demi-upgrade.service` is that separate privileged runner.
* **Coolify** shows *Update available* in its navigation and runs its
  `upgrade.sh` on the host as root, detached from the app it replaces; it
  refuses a downgrade and a second upgrade while one runs, and reads the
  progress back from a status file that the script writes
  ([`UpdateCoolify.php`](https://github.com/coollabsio/coolify/blob/main/app/Actions/Server/UpdateCoolify.php),
  [`upgrade.blade.php`](https://github.com/coollabsio/coolify/blob/main/resources/views/livewire/upgrade.blade.php)).
  It reaches the host over SSH with a root key; Demi asks systemd instead,
  so no key to the machine's root exists for the backend to lose.
* **macOS Software Update** names *Update Now*, *Your Mac is up to date*
  and *Last checked*. The page is *Updates*, shorter, as Home Assistant's
  and JetBrains IDEs' settings name theirs.

## Alternatives

* **A `sudo` rule for the `demi` user.** The backend's unit bounds its
  capabilities to binding a low port, so `sudo` cannot gain root under it,
  and a program the backend starts is killed with the backend's control
  group when the upgrade stops it.
* **Through the machine manager**, which runs as root and which the backend
  already reaches: the upgrade stops the manager too, and it would turn a
  program that runs Clouds into one that replaces the server.
* **A status file** that `demi-server` writes and the backend reads, as
  Coolify's: the backend would poll it, could miss the step before the
  stop, and would need a directory only root writes and the backend reads.
* **Automatic updates.** Not now: every update stops Demi for everyone, so
  an administrator chooses the moment.
* **A nightly release**, a pre-release of the newest commit published on a
  schedule, for a channel named Nightly. Releases stay what a person asks
  for; a scheduled one can come later as a channel of its own.
* **Beta** for the channel, as macOS and Home Assistant name theirs: a beta
  is a release on its way to stable, tried before it is published, and a
  deploy's pre-release is not yet that.

## Open questions

None.
