---
status: proposed
---

# A server takes whole releases of the channel it names

## Problem

A server release holds the backend, the web app and the Cloud image, but not
the programs Demi runs on a Host: the runner and the command programs, such
as `demi-file` and `demi-mobile`, for each of the six targets. Its
`release.json` names the GitHub release that offers them, and the backend
downloads each one the first time a Host of its target needs it.

So an update that Settings › Updates shows as done is not done. Say an
administrator updates a server to 0.1.5500 and, an hour later, pairs a
Windows laptop:

1. The laptop's installer asks the backend for the Windows runner.
2. The backend downloads it from GitHub, while the installer waits.
3. When GitHub is slow, limits the server, or is unreachable from it, the
   pairing waits on it or fails. Nothing on the Updates page or in the
   update's progress said that this download was still to come.

The same happens to a conversation's first command on a Host whose target
the backend has not stored yet, and to the paired devices that download the
next runner while an update prepares. The backend carries code for it all:
a download with its redirects, size limits and logs; a worker that stores
the programs of the known Hosts' targets ahead; and a record, `release.json`,
that only says where the programs are.

Installing does not name a channel either. `https://demi.sh/install.sh`
redirects to the `install.sh` of GitHub's latest release, the one Demi
published last. Demi has published none, as every release so far is a
beta, so the documented command answers 404 on every machine. Its
`--prerelease` installs what Settings › Updates calls Canary, under another
name, and nothing installs Beta.

## Decision

### What the user sees

The update downloads all of the release, and nothing after it does:

```text
Downloading Demi 0.1.5500
[==========          ]
120 MB of 300 MB · 12 MB/s · Less than a minute remaining
```

* A release downloads about 300 MB instead of about 130 MB: its programs for
  the six targets add about 170 MB, compressed.
* The server keeps the release it runs and the one before, so the programs
  take about 340 MB more of its disk. The backend's store keeps only the
  programs its Hosts used, as it does today.
* Once the update is done, pairing a Host of any target, a runner's update
  and a command's first run take their programs from the server's own
  disk. A server that cannot reach GitHub after its update works as it did
  before it.
* Installing a server downloads the same archive, so a new server needs
  GitHub once, for the install.

### How it works

1. **The archive.** `demi-<version>-server-linux-<arch>.tar.zst` holds a
   `files/` directory in its root, with the compressed copy of every
   runner and command program for every target: the files that a GitHub
   release offers one by one today.

   ```text
   /opt/demi/releases/0.1.5500/
     bin/  systemd/  web/  runners/  commands/  image/
     files/
       demi-runner-aarch64-apple-darwin.zst
       demi-runner-x86_64-pc-windows-msvc.exe.zst
       demi-file-aarch64-apple-darwin.zst
       ...
   ```

2. **The record goes.** The backend reads the programs from its root's
   `files/`. `release.json`, `ServerRelease` and `FilesLocation` go, and so
   does the URL source of `Sourcing`, with its download, redirects, size
   margin and logs. A developer's root, which `xtask server-release`
   assembles, has its own `files/` as well.
3. **The store stays.** The first time a Host needs a program, the backend
   copies it from `files/` into its object store, as it copies one from a
   developer's directory today: one copy per program, checked against its
   size and SHA-256, while other needs of the same program wait for it. A
   copy from the local disk takes milliseconds, so the worker that stores
   the known Hosts' programs ahead goes, and with it `foresee` and
   `store_ahead`. A store on S3 receives each program the same way, from
   the local disk instead of from GitHub.
4. **The next release.** While an update prepares, the paired devices
   download the next runner from the backend, which takes it from the next
   release's `files/`: `demi-server` has unpacked that release already.
5. **The GitHub release.** It no longer offers the programs one by one. It
   offers the server archives, the Cloud images, `demi-server`, `SHA256SUMS`
   and the debug symbols.
6. **What an older release meets.** A release before this one keeps its
   `release.json` and downloads its programs from its own GitHub release,
   which keeps them. While an update prepares, the running backend has the
   paired devices download the next runner, reading the next release's root:
   a backend before this release finds no `release.json` in this release's
   root, and this release's backend finds no `files/` in an older one's. In
   both cases the update goes on, and the devices download their runner
   after the switch, once, as `demi-server` prints: the first update to this
   release, and a rollback from it to an older one.
7. **Publishing.** A release publishes a build whose root holds `files/`;
   one made before this change is refused, since its root names programs
   the release no longer offers one by one.

### Installing names its channel

Demi has no stable release yet, so an install names its channel, as
Settings › Updates does, or a version:

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --beta
curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --canary
curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --version 0.1.5500
```

* Without one, `install.sh` installs nothing and says so:

  ```text
  install.sh: Demi has no stable release yet: choose a channel.
    The releases Demi publishes:  curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --beta
    The newest build:             curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --canary
  ```

* `--beta` installs GitHub's latest release; while Demi has published none,
  it says so and points to `--canary`. `--canary` installs the highest
  version among all the releases, as `--prerelease` did, which goes.
* `install.sh` is one script for every release, no longer an asset of each:
  it finds the version itself. `https://demi.sh/install.sh` redirects to
  `crates/server/install.sh` on the repository's branch that releases are
  published from.
* `demi-server upgrade` and `demi-server setup` take the same `--beta` and
  `--canary`, and `--prerelease` goes from them too. `upgrade` with neither
  and no version refuses with the same message; `setup` without one sets up
  its own release, which `install.sh` chose.
* The channel the backend offers in Settings › Updates stays Beta until an
  administrator changes it, however the server was installed.

## Spike

None: the archive's contents and the store's local source exist, for a
developer's build.

## Prior art

* **Coder** builds a *fat* `coder` binary that embeds the agent binaries
  for every operating system and architecture, compressed as
  `site/bin/coder.tar.zst`, unpacks them when the server starts, and serves
  them at `/bin/`, so a workspace takes its agent from its own deployment,
  air-gapped ones included
  ([coder/coder#2533](https://github.com/coder/coder/pull/2533),
  [`site.go`](https://github.com/coder/coder/blob/main/site/site.go)).
  Its *slim* binaries, without them, are for the workspaces. Demi does
  the same with its server archive.
* **VS Code**'s remote development downloads the server for each host
  from Microsoft's CDN when the host is first opened: it has no server of
  its own to hold them, which Demi has.

## Alternatives

* **Keep downloading on first need.** It saves about 170 MB per update and
  a few hundred MB of disk, and keeps every failure above out of the
  update's sight.
* **Download, during the update, only the targets of the paired Hosts.**
  A Host of a new target still needs the download on first need, so both
  paths stay.
* **Copy every program into the store as the backend starts.** It spares
  the first copy, which takes milliseconds, and costs the disk of the
  targets nobody uses, and on S3 an upload of 170 MB before each release
  serves.

## Open questions

None.
