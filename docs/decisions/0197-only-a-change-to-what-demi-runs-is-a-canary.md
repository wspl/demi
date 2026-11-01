---
status: proposed
---

# Only a push that changes what Demi runs is a Canary

Supersedes: 0143 (Every push to main that builds and passes its checks is a
Canary), the part that a push changing only documents or the website is one,
and the part that CI cancels its run of a commit when a newer one is pushed.

## Problem

Alex runs a server on Canary. At 10:00 a developer fixes a typo in
`README.md` and pushes it to `main` alone. The push builds every target,
about fifteen minutes of six platforms' runners, passes its checks, and is
published as `v0.1.4214`. Alex's server finds it within the hour, and the
sidebar shows the green update icon. Alex updates: the server stops and
restarts, Clouds wake with the new programs, and the paired devices download
their runners again, for programs whose only change is their version number.
A push that changes only the website does the same, though the website
deploys on its own (`site.yml`) and is in no release.

## Decision

A push to `main` that changes only files no release holds builds nothing, so
it is no Canary. Its checks still run, and the website still deploys:

```text
push to main ─┬─► Build: only when it changes a file a release holds
              ├─► CI: always (gitleaks reads every pushed commit)
              └─► Site: always
Canary: both Build and CI succeeded for the commit
          └─ no Build run for it ─► nothing to publish
```

The files no release holds, `paths-ignore` of `build.yml`'s `push`:

* `docs/**`: the overview, guides, decision records and the brand's assets.
* The website's sources, pages, assets and settings under `packages/site/`,
  but not its `package.json`: the web build's `bun install` reads every
  workspace's manifest.
* The repository's documents at its root: `README*.md`, `AGENTS.md`,
  `CLAUDE.md`, `ARCHITECTURE.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`
  and `SECURITY.md`.
* `.demi/**`: the workflows and profiles of the agents that work on this
  repository.

`LICENSE`, `LICENSE-DEMI` and `NOTICE` are not in the list: a server release
holds them. No workflow is in it either, the website's included: a release
that has to build a commit again refuses one whose `.github` differs from the
branch's newest (`release.yml`), so a push that changed only a workflow must
build. A file that a build starts to read is taken out of the list in the
same work package.

GitHub compares the tree the push leaves with the tree before it, so a push
of several commits builds when the two differ in a file a release holds, and
then builds its newest commit, as today. A push of more than 1,000 commits,
or one whose comparison times out, always builds. Versions still count every
commit, so a commit that is no Canary only leaves a gap in the numbers, as
busy hours already do.

CI keeps running on every push, since gitleaks reads only the commits the
push brings, and on `main` it no longer cancels a run when a newer push
comes. Today a code push A whose CI is still running is saved by the next
push B: B cancels A's checks, but B builds and passes, and B's Canary holds
A's changes. When B changes only documents, B builds nothing, so A, with its
checks cancelled, would wait for the next push that builds. Each push to
`main` now has its own concurrency group, the commit's SHA; a pull request
keeps one group per pull request, which cancels its older run.

`canary.yml` needs no change: it already publishes nothing for a commit
whose build has not succeeded.

A deploy (`upgrades.md` § A development build on a server) still publishes
the newest commit of `main`. Right after a push of documents, that commit has
no build, so the deploy builds it, as it builds any commit without one, and
publishes a Canary whose programs differ only in their version. Choosing the
newest built commit instead depends on GitHub having registered the build of
a push that just landed, so a deploy could quietly take an older commit.

## Spike

None: `paths-ignore` is GitHub's filter for this, and a workflow it skips
has no run, which `canary.yml` already reads as "not passed".

## Prior art

* **uv** (`astral-sh/uv`, `.github/workflows/plan.yml`) skips its code tests
  and the builds they need when every changed file is under `docs/` or ends
  in `.md`; its other checks still run.
* **GitHub Actions** documents `paths-ignore` on `push` for skipping a
  workflow by the files a push changes, comparing the tree after the push
  with the tree before it.

## Alternatives

* **Skip CI as well**: saves its runs too, but gitleaks scans only the pushed
  commits, so a secret pushed in a commit of documents would never be
  checked.
* **Decide in a first job of `build.yml`** whether to build: the list would
  live in a script rather than in the trigger, and the skipped run would
  still succeed, so `canary.yml` would need a second rule to tell it from a
  build.
* **Compare the release's programs with the last Canary's** and publish only
  when they differ: every build stamps its version into the programs and the
  web app takes a fresh build id, so they always differ, and it would still
  build every push for fifteen minutes to find out.

## Open questions

None.
