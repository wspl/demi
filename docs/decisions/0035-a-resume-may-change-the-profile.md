---
status: accepted, superseded in part by 0043
---

# A resume may change the profile

## Context and Problem Statement

An agent's subagent stopped on its model's usage limit, halfway through its
work. To go on with another model, the agent forks it onto another profile:
a new child, with a new number, a copy of the history up to the source's
last completed answer, and the source left behind as failed. When the
source never completed an answer, the fork starts with nothing at all.
`demi agent resume` keeps the child and its whole history, but only on the
profile it ran on.

## Considered Options

* Keep forking to change a profile.
* Let a resume name the profile its next round runs on.

## Decision Outcome

Chosen option: a resume may name a profile, because going on with the same
work on another model is the same child going on, not a new one.

* `demi agent resume <id> --profile <name>` gives the archived child its next
  round on that profile: its model, effort, context limit and instructions.
  It keeps its number, its description and its whole history, the steps of a
  round that failed included. Without `--profile` it runs on its own profile,
  as today.
* What a child runs on is its record's, the one owner of its model. A resume
  with a profile, and a fork, write the profile's model, instructions and
  Context into the record in the same commit that creates or reopens the
  child. A checkpoint's model is only what its history was last requested
  with, so a round that starts, restarts or restores with a record that names
  another model switches to it at its first request, compacting first when
  the history is over the new model's window. A round stopped or failed
  before that request leaves the next resume, with or without `--profile`,
  on the profile's model and instructions together.
* A child's instructions and Context are its record's as well: one that
  inherited its parent's profile records the parent's own at its spawn, and
  a later change of its parent's profile reaches it in no round.
* The provider of the model a history was last requested with is not a
  condition of going on. With it gone, a resume, a restore and a round after
  a stopped switch start on the record's model. Only a history over the new
  model's window needs the old model, to compact it first; then the round
  fails and says that the old model cannot be asked, and why.
* The Subagents switch guards the making of children. A resume that names a
  profile reads that profile's settings and goes on while the switch is off,
  as one that names none does. Its refusals say what a resume does, not what
  a spawn does.
* The history goes to the new model as a fork's does: what one vendor's
  model wrote is replayed as the other's can read it.
* The child's row and `demi agent show` name the profile it now runs on; its
  earlier rounds keep the model they ran on.
* `demi agent fork` stays for a second child from what one knows, the source
  going on or kept as it was.

### Performance

* The first request after a change of model reads the whole history
  uncached: one cache write of its size, as a fork's first request is.
* Nothing else changes.

### Consequences

* Good, because a child that failed on one model goes on, on another, with
  all it did, under the number the parent already knows.
* Bad, because a resume now has two ways to run, its profile's and another's,
  which its help names.
