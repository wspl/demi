---
status: proposed
---

# A server prepares its next release ahead

Supersedes: 0106 (An administrator updates the server from Settings), the
daily check; and 0122 (An update waits for no device), the part that has
the devices download the new runner from Update Now on.

## Problem

Alex is an administrator of a server on the Canary channel. Demi looks for a
newer release once a day, so a fix published in the morning shows in
Settings › Updates the next day. When Alex clicks **Update Now**, the server
first downloads the release, unpacks it, downloads gVisor and imports the
Cloud image, about a minute of work that needed no decision, while Alex
waits in front of the list. The devices start downloading their new runner
only then, too.

## Decision

The server looks for a newer release every hour, and gets the newest one
ready as soon as it finds it, without asking. **Update Now** is left with
what needs the administrator: two quick checks and the restart.

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Demi 0.1.5500 is available                             [Update Now]  │
│ Downloading in the background: 88 MB of 214 MB                       │
└──────────────────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────────────┐
│ Demi 0.1.5500 is available                             [Update Now]  │
│ Ready to install. You have 0.1.5469.                                 │
└──────────────────────────────────────────────────────────────────────┘
```

- **Every hour** the backend looks for the newest release of its channel,
  and once as it starts, as today. **Check Now** stays.
- **Ready ahead.** When the newest release is newer than the one the server
  runs, the server downloads and unpacks it, downloads its gVisor and
  imports its Cloud image beside the running one, and the paired devices
  download its runner and package programs. None of it stops or changes
  anything that runs. The row's second line says how far it is:
  *Downloading in the background: 88 MB of 214 MB*, *Installing in the
  background…*, then *Ready to install. You have 0.1.5469.*
- **A failure** says *Couldn’t download Demi 0.1.5500 in the background.
  You have 0.1.5469.*, and the next hourly check tries again. Update Now
  still works: it does what is missing.
- **Update Now** lists the same seven steps as today. What was done ahead
  shows at once as *Already installed*; the configuration's and the
  databases' checks run, and Demi restarts. Clicked while the work ahead
  still runs, it takes that work over: the list shows its steps where they
  are and goes on from them.
- Nothing is offered to turn this off: every server gets its next release
  ready the same way.

### What stays on the disk

The work ahead adds one release to what a server and a device keep, never
more. A newer release replaces the one prepared before it.

| Where | Kept |
| --- | --- |
| The server's `releases/` | The current release, the one a rollback returns to, and the one prepared ahead |
| The server's gVisor versions | Those of the same three releases |
| The machine manager's Cloud bases | The configured base, the previous one, the one prepared ahead, and any a Cloud is pinned to, as today |
| A device's runner `releases/` | The running executable and the one downloaded ahead |
| A device's package cache | The programs of the running release and those of the release ahead |

- The server's preparation of a newer release removes the earlier prepared
  release and its gVisor version, unless the current or the rollback release
  uses them, and the Cloud base imported ahead for it, which no manager ran
  and so no Cloud is pinned to.
- A device's download ahead of a newer release removes the executable and
  the programs of the earlier release ahead, unless the running release
  uses them.
- An update keeps pruning as today: the release the server leaves becomes
  the rollback one, and a device's new runner removes every earlier
  executable and program.

### How it works

- `CHECK_EVERY` becomes one hour. A check is one request to GitHub, far
  below its 60 requests an hour per address.
- After every check that finds a newer release, the backend writes its
  version to a second request file, `/run/demi-upgrade/prepare`. A path unit
  of the release, `demi-prepare.path`, starts `demi-prepare.service`, which
  runs `demi-server prepare --requested` as root.
- `demi-server prepare` fetches the release as `upgrade` does, then hands
  over to that release's own `demi-server prepare`, which downloads its
  gVisor, imports its image and tells the running backend over the
  management socket that the release comes next, as the prefetch does
  today. A release already prepared costs a few checks and tells the
  backend again, so a backend that restarted meanwhile hears of it within
  the hour. Its steps reach the backend as notices of their own, which the
  row's second line shows; they never show as an update that runs.
- `demi-server upgrade --requested` waits for the lock that a running
  `prepare` holds, instead of failing, and finds its work done. A `prepare`
  that finds the lock held removes its request and ends, and the next check
  asks again.
- A run that hands over to a newer release's `demi-server` passes the lock
  it holds to it, so no other run, such as `prepare`, takes the lock between
  the two and removes the release the move goes to.
- `prepare` waits at most a minute for the backend to answer its
  announcement of the runner release, so a backend that does not answer
  never holds the lock, and an update behind it, for longer.
- `demi-machine-manager --import` records the base it imported beside a
  running manager in the store's `ahead.json`, and removes the one recorded
  before it, unless a manager is configured with it. A manager configured
  with the recorded base removes the record, and keeps the recorded base
  when it removes the bases nothing needs.
- A runner downloads one release ahead at a time: a newer one stops the
  download of an earlier one, and once its programs are in place, the
  executable and the programs of any earlier release it downloaded ahead
  go, the running release's staying.

### What an older server or runner meets

A server of an earlier release has neither the units nor the request file,
and keeps checking daily, until it updates to this release, whose move
installs the units. A runner of an earlier release takes the prefetch as it
does today, and keeps the earlier prefetches until it updates.

## Spike

None: the fetch, the image's import, gVisor's download and the devices'
prefetch exist; what changes is when they run and what each removes.

## Prior art

- macOS Software Update downloads and prepares an update in the background
  when *Download new updates when available* is on, which it is by default,
  and asks only to restart.
- [Home Assistant](https://github.com/home-assistant/supervisor) checks for
  new versions on a schedule and installs only when asked.
- Chrome downloads and stages its next version while it runs, and applies it
  at the next relaunch.

Demi takes macOS's default without its switch: a server's download costs
nothing a user waits for, and one behavior is easier to reason about.

## Alternatives

- **Download ahead, prepare at Update Now.** The download is most of the
  wait, but the image's import is the rest of it, and it changes nothing
  that runs.
- **Keep every release prepared.** On Canary, a server that is not updated
  for weeks would keep a release for every build.

## Open questions

None.
