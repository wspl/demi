---
status: proposed
---

# Every push builds, and a release publishes a build

## Problem

A batch is merged and the user asks for a deploy. The deploy commits the
next version, starts the release workflow, and the server waits for it:
a quarter of an hour on a good day, and much longer when a download or a
runner is slow. Every deploy waits for a whole build that could have run
while the work was merged. Each version is also a commit of its own, which
exists only to change one number.

## Decision

**Every push to `feat/demi-next` builds everything a release publishes,
and publishes nothing.** A release, or a deploy's pre-release, publishes a
finished build of the commit it names, which takes a few minutes.

```text
push ─► build.yml: programs, server, web, server release (root, files,
        │          image) ─► the run's artifacts, version 0.1.<commits>
        │
gh workflow run release.yml ─► finds that commit's build
        ├─ finished with its files ─► publishes them as v0.1.<commits>
        ├─ running or waiting ──────► waits for it, then publishes
        └─ none, failed, or expired ► builds the commit, then publishes
```

**A version is the number of commits in the commit's history**, made by
the build: `0.1.4213` for a commit with 4,213 commits behind it, counted
with `git rev-list --count`. The same commit always has the same version,
and a later commit on the branch always a higher one. No commit changes a
version any more; versions skip numbers between releases. The tag
`v0.1.4213` on the published commit still says which version a server
runs.

**No build is cancelled.** A build that runs finishes. Of the pushes that
arrive while one runs, only the newest waits; it takes the place of the
one that waited before. So the branch's newest commit is always built, and
a commit pushed in a busy hour may never be.

The design to build from:

* `build.yml` starts on a push to `feat/demi-next`, or by hand on a
  branch, with the concurrency group `build-<ref>` and its default single
  waiting place, `cancel-in-progress: false`. A call from `release.yml`
  has a group of its own, so it waits for no build of a push and replaces
  none.
* Its version job reads the history (`fetch-depth: 0`, `filter: tree:0`)
  and outputs `0.1.<count>`, the build version. Each job that compiles
  writes it into its checkout's `Cargo.toml` and `Cargo.lock` after the
  Rust cache is restored, and never commits it. The workspace version,
  the one committed in `Cargo.toml`, stays what it is, and a developer's
  build reports it as its own build version.
* A build saves a Rust cache entry only when the dependencies changed, and
  only from a job that succeeds. An entry holds a fingerprint of what
  decides the compiled dependencies: the packages of `Cargo.lock` that
  come from a registry or git, the vendored crates, the root manifest's
  profiles, patches and workspace dependencies, the toolchain and Cargo's
  configuration. A job that restores an entry with its own fingerprint
  saves nothing. Without it, the cache action's own
  key, which covers the manifest of every crate of the workspace, names a
  new entry for each push that, for example, adds `tracing.workspace =
  true` to a crate that did not use it, though `Cargo.lock` already holds
  `tracing`, and each such entry is 1 GB. The repository's cache is billed
  beyond 10 GB, and builds run on every push.
* The jobs and their order are today's release jobs. `release.yml` has
  three: one finds the commit's build, one calls `build.yml` for a commit
  with none, and one publishes. It has the inputs `commit` (default: the
  branch's newest) and `prerelease`. It finds a finished build by
  `gh run list --commit` among the branch's runs, takes its files with
  `actions/download-artifact` from the build's run, by `run-id`, and waits
  for a build that runs. The commit must be in the history of the branch
  the release starts on: a version counts one line of history, so a commit
  of another branch would take the version of the branch's commit with
  the same count, and a build of another branch carries that branch's name.
* A build counts as finished only when every artifact the release
  publishes, the debug files included, is kept for two more hours, since
  each expires one day after its own upload and a download by pattern
  skips an expired one without an error. The release also checks that
  every asset is there before it creates the release, which cannot be
  undone.
* The release calls the `build.yml` of the branch's newest commit, whose
  sources and actions are those of the commit it builds. It builds an
  older commit only when `.github` is the same in both.
* A build's artifacts are kept one day, as today: storage is billed by the
  hour, and a release that finds them gone builds again.
* A release refuses a version that has a release already, as today.
* A release builds only a commit that has the actions of this decision, so
  a commit from before it is never published.
* `docs/delivery/upgrades.md` § A development build on a server says the
  new steps.

## Spike

None.

## Prior art

* GitHub's reusable workflows and cross-run artifact downloads are the
  documented way to build once and publish later
  ([reusing workflows](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows),
  [download-artifact `run-id`](https://github.com/actions/download-artifact#download-artifacts-from-other-workflow-runs-or-repositories)).

## Alternatives

* **A build number, `0.1.<run number>`.** A new workflow's run numbers
  start at 1, below the versions already published, so the series would have to
  move or carry an offset; and two runs of one commit would differ.
* **Bump the version after each release.** Every release adds a commit
  that only changes a number, as today.
* **Build every push, in order.** A dozen pushes in a busy hour
  would queue hours of builds, and a deploy would wait for all of them.

## Open questions

None.
