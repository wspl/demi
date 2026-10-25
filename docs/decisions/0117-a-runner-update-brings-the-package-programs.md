---
status: accepted
---

# A runner update brings the package programs

## Problem

After a server upgrade, the first time Alex opens the browser panel on a
laptop, the panel shows "Starting the browser…" for five seconds or more. The
runner is downloading the new `demi.browser` program (26 MB) from the server,
and nothing says so. Every package program (`demi.browser`, `demi.mobile`,
`demi.fetch`, `demi.file`) is new in each release, so each one makes its
first use after each release wait this way, in a panel or in an agent's
command. The runner downloads it then because it learns of a package only
from the work that runs it.

## Decision

A Host has the programs of its release's packages before anything needs one.

- **Once a runner connects**, the backend sends it the command packages of its
  release (`programs`). The runner puts the program of each for its target in
  its artifact cache in the background, one at a time; one that is there
  already costs nothing, and the cache removes the older versions as an
  install always did.
- **Ahead of an upgrade**, the `prefetch` that has a device download the next
  runner release names that release's command packages too. The runner puts
  their programs in its cache after its own executable and answers
  `prefetched` once all are in place, so the upgrade's wait for the devices
  covers them. The release's programs and the current ones are kept side by
  side until the runner moves to the release: no install of either removes
  the other's, and the new runner's first installs remove the older.
- The backend locates these programs as it locates a job's: the runner asks
  under the new artifact owner `programs`, of the backend's release or, with
  `next`, of the release the prefetch named, and the backend answers for the
  packages it sent. A backend with a local store serves the next release's
  programs at the same address as its own.
- A program that could not be fetched fails neither the connection nor the
  prefetch: its first use downloads it, as before, and the Host log says why
  (`package_program.keep_failed`). A backend that cannot read the next
  release's packages, as of a newer package protocol, prefetches the runner
  alone (`runner_release.programs_unread`).

```text
backend                                  runner
  |-- programs {packages} --------------->| keeps each program, in the
  |<-- artifact_location {programs} ------|   background
  |-- prefetch {release, exe, packages} ->| executable, then the programs,
  |<-- artifact_location {programs,next} -|   keeping the current ones
  |<-- prefetched ------------------------|
```

Every Host downloads every program of its target with each release, about
60 MB, whether it uses the package or not.

## Spike

None: the cache, the backend's sourcing and the locating of a job's
artifacts were reused as they are.

## Prior art

VS Code's remote server is one archive with its built-in extensions: an
update brings them, and nothing downloads on their first use.

## Alternatives

- **Download each program on first use and show its progress.** Every
  release would make every first use wait for something the runner was sure
  to need.
- **Install the programs only when a runner connects.** A device would
  download them in the first seconds after an upgrade, the moment people come
  back to it.

## Open questions

None.
