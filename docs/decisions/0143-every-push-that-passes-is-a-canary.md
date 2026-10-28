---
status: proposed
---

# Every push to main that builds and passes its checks is a Canary

Supersedes: 0076 (Every push builds, and a release publishes a build), the
part that a push publishes nothing.

## Problem

Alex runs a server installed with `--canary`, the channel that, as the README
says, gets every build first. At 10:00 a fix for a bug Alex hit is merged into
`main`. The push builds, and nothing more happens: a build becomes a release
only when a developer deploys to their own server, which publishes it as a
pre-release on the way. If nobody deploys that day, Alex's Settings › Updates
says *Demi is up to date* until someone does, though thirty merges may be
waiting. Canary gets the builds a developer happened to deploy, not every
build.

## Decision

A push to `main` whose build succeeds and whose checks pass is published as
a pre-release, the next Canary, without anyone starting it:

```text
push to main ─┬─► Build (build.yml) ──┐  each one that completes starts
              └─► CI (ci.yml) ────────┴► Canary (canary.yml)
                                           both succeeded, and no release
                                           of this or a newer version?
                                             └─► release.yml, prerelease
```

* A server on Canary offers the new release in Settings › Updates about
  twenty minutes after the push, once the backend's hourly check, or **Check
  Now**, finds it. Beta is unchanged: it offers a Canary only after someone
  publishes it with `gh release edit "v$version" --prerelease=false
  --latest`.
* Not every commit becomes a Canary, as not every commit is built today. A
  build that runs takes the newest push that arrived meanwhile, and CI
  cancels its run of a commit when a newer one is pushed, so a busy hour
  publishes its newest commit that passed.
* A commit whose build or checks failed is never published. The next push
  that passes is.

The design to build from:

* `.github/workflows/canary.yml` starts on `workflow_run` of `Build` and
  `CI`, `types: [completed]`, `branches: [main]`. Its job reads the run's
  `head_sha`, finds that commit's push runs of both workflows, and stops
  with success unless both concluded `success`: whichever of the two
  completes last publishes.
* It works out the version, `0.1.$(git rev-list --count "$commit")`, and
  stops when a release of that version, or of a higher one, exists, so an
  older commit never comes after a newer one. Otherwise it starts
  `release.yml` on `main` with the commit and `prerelease=true`
  (`actions: write`; a workflow may start another by `workflow_dispatch`
  with the repository's token), and waits for it.
* Its concurrency group, `canary`, queues runs and cancels none, so the two
  completions of one commit never start two releases: the second finds the
  release the first made.
* A deploy keeps its steps (`upgrades.md` § A development build on a
  server). It usually finds the Canary of the newest commit already
  published and skips publishing. When it starts the release while the
  Canary starts the same one, the release that comes second refuses the
  version, and the deploy goes on when `gh release view "v$version"`
  succeeds.
* Every Canary is kept, as every version is today. A release takes about
  2.9 GB, its debug files included, and at today's pace about a dozen
  publish on a busy day; GitHub sets no limit on a release's size or
  downloads.

### Schemas

Any Canary may hold the data of a server, so every published release enters
the schema history, not only the ones a developer deployed
(`crates/backend-database/src/schema.rs`):

* The first change to a schema after a release adds that release's schema to
  the history, with its migration, in the same work package.
* Every later change before the next release extends that migration.
* Since a Canary follows nearly every merge, a work package that changes a
  schema adds a step in most cases. It reads the newest release's schema
  with `git show "v$version":<path>` to know whether its starting point has
  shipped.

`upgrades.md` § A development build on a server says "once per deploy"
today; it says "once per release" afterwards. The Data check of an upgrade,
which refuses a database whose schema is neither the new one nor in its
history, is unchanged and still catches a step that was left out.

## Spike

None: each step uses what the workflows already do. The only new event is
`workflow_run`, which GitHub documents for acting on another workflow's
result.

## Prior art

* **Bun** publishes a canary build of every commit to `main`, which
  `bun upgrade --canary` installs.
* **Deno** builds a canary from every commit to `main`, which
  `deno upgrade --canary` installs.
* **Chrome** Canary ships a build of the trunk every day, and Beta takes a
  build that has been on Canary.

Demi does as Bun and Deno, and adds the condition that the commit's checks
passed, since its build runs none.

## Alternatives

* **A nightly Canary**, once a day by schedule: fewer releases and fewer
  schema steps, but a fix waits up to a day, and the channel's promise,
  every build first, would still not hold.
* **Publish every build without waiting for CI**: a few minutes sooner, but
  a commit whose tests failed would reach every Canary server.
* **Publish from a last job of `build.yml`**: that job would have to wait
  for CI inside the build, which no other job does; `workflow_run` is
  GitHub's way to act on two workflows' results.

## Open questions

None.
