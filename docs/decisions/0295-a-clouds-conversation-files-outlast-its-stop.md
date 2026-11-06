---
status: proposed
---

# A Cloud's conversation files outlast its stop

Supersedes: 0193 (A conversation's browser keeps its sign-ins), the part
that places a conversation's browser profile under `<DEMI_HOME>/scratch`
on every Host.

## Problem

Alex asks the agent on a Cloud to check the orders on `shop.example.com`,
and signs in to the shop in the work panel. The agent saves a probe script
and a few screenshots in `$DEMI_SCRATCH`. Alex comes back the next morning:
the Cloud stopped overnight after its idle window, the shop opens signed
out, the script and the screenshots are gone, and so are the recordings of
the browser.

A conversation's scratch directory, its recordings and its browser profile
are promised to last as long as the conversation: until it is deleted for
the scratch files, until it is archived for the others. They live in
`<DEMI_HOME>/scratch/<conversation>/`. On a paired computer `DEMI_HOME` is a
directory of the user's home, so they do. On a Cloud, `DEMI_HOME` is
`/run/demi`, the runner's state for one boot, in memory, so every stop
loses them.

## Decision

On a Cloud, the scratch root is `~/.demi/scratch` in the Cloud user's home,
which a stop and a reset keep. Every other Host keeps
`<DEMI_HOME>/scratch`.

```text
/home/demi/.demi/
  scratch/<conversation>/       the agent's files, one per conversation
    .recordings/
    .browser/                   the conversation's own Chrome profile
```

What changes for the user: on a Cloud, sign-ins in the conversation's own
browser, the agent's scratch files and the recordings outlast the Cloud's
stop and reset, as on a paired computer. They count against the Cloud's
home disk, as everything in the home does, and go when the conversation is
archived or deleted, as today.

### Design

The runner owns the scratch root. It picks it once, at start, and every
program it starts reads it rather than deriving it from `DEMI_HOME`:

- `Scratch::new` takes the root itself: `~/.demi/scratch` on a managed
  boot, `<DEMI_HOME>/scratch` otherwise.
- The runner gives jobs and residents `DEMI_SCRATCH_ROOT`, beside
  `DEMI_HOME`, and still names the root in its hello.
- `demi_command_protocol` gives `scratch_root()`, which reads
  `DEMI_SCRATCH_ROOT`, and `scratch_directory()`, a conversation's
  directory under it, or under the system's temporary directory for a
  program the runner did not start, as today. The recordings', the browser
  profile's and the mobile resident's directories derive from them instead
  of from `DEMI_HOME`.

What was there: a Cloud that upgrades to this release leaves its
`/run/demi/scratch` behind, which its next stop clears; nothing moves. A
Cloud's conversation that ran before the upgrade starts with an empty
scratch directory, as it would have after a stop. A command still running
across the upgrade keeps the `DEMI_SCRATCH` it started with, in
`/run/demi`, so what it saves there goes at the next stop, as before.

## Spike

None.

## Prior art

- GitHub Codespaces keeps only `/workspaces` across a rebuild of the
  container, and its documentation tells users to keep there what must
  last
  ([Rebuilding the container](https://docs.github.com/en/codespaces/developing-in-a-codespace/rebuilding-the-container-in-a-codespace)).
- Gitpod's workspaces keep only `/workspace` when they stop
  ([Workspace lifecycle](https://www.gitpod.io/docs/configure/workspaces/workspace-lifecycle)).

Demi already promises the conversation's files last as long as the
conversation, so they go to the one directory of a Cloud that persists, the
home.

## Alternatives

- Only the browser profile moves to the home. The scratch files and the
  recordings would still be lost at every stop, against the same promise.
- `/run/demi/scratch` becomes a link into the home. Every program would
  still derive the root from `DEMI_HOME` in its own way, and the link would
  be one more thing a Cloud's boot must make before any job runs.
- The whole runner state moves to the home. It holds the runner's lock and
  its per-boot state, which must start empty at each boot.

## Open questions

None.
