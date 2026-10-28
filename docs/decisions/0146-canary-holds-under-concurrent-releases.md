---
status: proposed
---

# Every schema main held enters the history, and a draft is not a release

Supersedes: 0143 (Every push to main that builds and passes its checks is a
Canary), the part on schemas, and its promise that an older commit never
comes after a newer one.

## Problem

0143 publishes builds while work goes on, and three of its rules assume that
nothing else happens in between.

**A schema is lost.** Alex merges A, which changes the control database's
schema from S0 to S1. A's checks pass and its build starts. Before the build
ends, Sam merges B, which changes the same schema again. The newest release
still holds S0, so, as 0143 says, B extends A's migration to S0 to S2 and
keeps no S1. A's build ends and is published, then B's. A server that took
A holds S1, which B's history does not know, so its upgrade to B stops at the
Data check, and every later release stops there too.

**A draft counts as a release.** `gh release create` makes a draft, uploads
the files, about 3 GB, and publishes the draft at the end. `gh release view`
finds a draft as well. A deploy that starts while the Canary of its commit
uploads finds the release, skips the wait and upgrades the server, which
cannot download the files of a draft.

**An older Canary can still come after a newer one.** While a deploy's
release of a newer commit B runs and has made no release yet, the Canary of
an older commit A sees no higher version and starts its release, which waits
behind B's and publishes after it. Installers and servers take the highest
version, so A is never offered over B, but 0143 says this does not happen.

## Decision

**Every work package that changes a schema adds the schema `origin/main`
holds to the history, with its migration**, named by the version of the
commit on main it was taken from. Each merge may become a Canary while a
later one is built, so any schema main held may be on a server. A schema
main held without a release adds a step that no server needed, which costs
only that step.

```text
main:   S0 ──A──► S1 ──B──► S2
history after B:  S0 → S1 → S2     (0143 would keep S0 → S2)
```

**A release is published only when it is not a draft.** The deploy's steps
and `canary.yml` ask GitHub whether the release is a draft
(`gh release view --json isDraft`), and wait while it is:

* A deploy that finds the Canary of its commit uploading starts its own
  release, which waits behind the Canary's in their concurrency group and
  then refuses the version; the deploy goes on once the release is no
  draft.
* `canary.yml` reads the highest published version from the repository's
  tags, `v<version>`, which a release gets when its draft is published, so
  neither a draft nor the order releases were made in counts.

**An older commit's Canary may be published after a newer one** when a
deploy publishes the newer one meanwhile. Nothing offers it over the newer:
`install.sh` and the backend take the highest version. `canary.yml`'s
comment says so instead of promising the order.

## Spike

* GitHub CLI's source: `release create` sets `draft: true` while it uploads
  assets and publishes the release after the last one
  (`pkg/cmd/release/create/create.go`), and `FetchRelease` finds a draft by
  its pending tag when no published release has it
  (`pkg/cmd/release/shared/fetch.go`).
* `install.sh`'s `newest` and `shared-artifacts`' `highest` take the highest
  `v<version>` tag of all releases.
* `build.yml` cancels no running build, so a commit whose checks passed and
  whose build started is published whatever merges after it. A newer push
  cancels an older commit's checks only while they run.

## Prior art

None needed: the change follows from GitHub's own draft behavior and from
the order of Demi's workflows.

## Alternatives

* **Publish only one release at a time across deploys and Canaries, with
  the version check inside it**: keeps the order but makes a deploy wait
  for a Canary's whole upload, and buys nothing a reader of releases sees.
* **Keep the schema rule and check the builds in flight**: a developer
  would have to read GitHub's queue before each schema change, and a build
  that starts after the check would still slip through.

## Open questions

None.
