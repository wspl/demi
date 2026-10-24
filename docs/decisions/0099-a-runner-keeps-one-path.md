---
status: proposed
---

# A paired device's runner runs from one path, so a Mac keeps its permissions

## Problem

On a paired Mac, an agent runs `ls ~/Desktop`. macOS asks whether
`demi-runner` may access files in the Desktop folder, and the user allows
it. The next day the backend is upgraded, the runner updates itself, and the
same command asks again; so do Documents, Downloads, Photos and iCloud
Drive, one dialog each, after every update.

The runner of each release lives in its own directory,
`<installation>/releases/<release>/demi-runner`, and an update starts the
new one from there. macOS's privacy controls (TCC) tell a program that is
not an app bundle by its path: the system log names the client as
`identifier /Users/alex/.demi/runner/<id>/releases/<release>/demi-runner,
type: 1`, type 1 being a path. A new release is a new path, so it is a new
program, with none of the permissions the user gave the old one. Its code
signature is not the cause: every release carries the same designated
requirement, `identifier "sh.demi.runner" and certificate leaf = H"…"`.

## Decision

**The runner always runs from `<installation>/demi-runner`**
(`demi-runner.exe` on Windows). An update replaces that file in place, so
on a Mac the permissions the user gave stay with it from one release to the
next. One layout serves macOS, Linux and Windows.

```text
  <installation>/
    demi-runner                    the runner, the only one started
    release-id                     its release
    run, run.ps1                   the launcher, which starts demi-runner
    releases/<release>/demi-runner a release downloaded ahead, until used
    releases/.download-*/          a download in progress
    releases/.replaced-*.exe       on Windows, a runner set aside
```

* **An update.** Under the install lock, the runner puts the new release's
  executable in `releases/<release>/` and checks it, as today. Then:
  - On Linux and macOS it renames that file over `<installation>/demi-runner`,
    which is atomic on one file system and leaves the running process its
    own file.
  - On Windows, where a running executable cannot be replaced but can be
    renamed, it renames its own file to `releases/.replaced-<release>.exe`
    and moves the new one in.
  - It writes the launcher and `release-id`, removes everything else in
    `releases/`, and starts the fixed path, in its place on Unix and beside
    itself on Windows, as today.
  - A failure before the rename leaves the runner on its release. A failure
    after it leaves the new executable named as the old release; the backend
    then offers the release again, and the update downloads it again, since
    `releases/<release>/` no longer holds it.
* **The installers** download into `releases/.download-*/` and check the
  file there. When they start a runner, they move it to
  `<installation>/demi-runner`, on Windows after setting aside the file
  there, which a runner that has just drained may still hold; when the
  release already runs, the download is discarded.
* **The launcher** starts `<installation>/demi-runner`. Its text has one
  owner, `runner-protocol`, from which both installers write it, and the
  runner too.
* **An installation from before.** Its runner of this release starts from
  `releases/<release>/demi-runner`, by an older launcher or an older
  runner's update. Under the install lock it links its own file to
  `<installation>/demi-runner`, writes the launcher anew, removes the
  releases, and starts itself from the fixed path. Until the launcher is
  written the old path still works, so a failure leaves a launcher that
  starts the runner, which tries again at its next start. A Mac asks once
  more, for the new path, and not after.
* **Removal** deletes `demi-runner` with the installation's other files.

## Spike

Does macOS keep a permission when the file at a path is replaced by another
build? A small program signed with a stable identity, its designated
requirement `identifier "sh.example.spike" and …`, was started by launchd
from a fixed path and read the Desktop folder; the user allowed it. A second
build, different bytes and the same identity, was renamed over the same path
and started the same way: `tccd` answered `authValue=2, authReason=2`, the
stored consent, and asked nothing. So a fixed path with a stable signature
keeps the permission.

## Prior art

* Sparkle, the updater of most Mac apps outside the App Store, replaces the
  app at its path, and macOS keeps its permissions by its identity there.
* The GitHub Actions runner keeps each version in `bin.<version>/` and
  points a `bin` link at the current one. A link would not help here: a
  process runs from the file the link resolves to, which is still a
  versioned path.

## Alternatives

* Install the runner as an app bundle, `Demi Runner.app`, which macOS tells
  by its bundle identifier wherever it is: the dialogs would name an app,
  but the release, the installers and the updates would all change, for the
  same result on permissions.
* A link from a fixed path to the versioned file: the process still runs
  from the versioned path, as above.
* Keep the layout and tell users to grant Full Disk Access: one grant per
  release all the same, and far wider than what the agent needs.
