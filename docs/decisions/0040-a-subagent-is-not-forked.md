---
status: accepted
---

# A subagent is not forked

## Context and Problem Statement

`demi agent fork` starts a new child from a copy of another child's history,
up to its latest completed answer, on a profile the parent names. It was
added to move a child's work to another profile. Since 0035, `demi agent
resume --profile` does that on the child itself, with its whole history,
the unfinished round included, and no new id. A parent that hands one
child's findings to another writes them into the new child's brief. Nothing
else uses fork.

## Decision Outcome

* `demi agent fork` is removed: the command, its help, its server
  operation and its tests.
* `demi agent resume --profile` is how a child goes on with another
  profile.
* A message's Fork in the page, which starts another conversation, is not
  affected.

### Performance

* No change.

### Consequences

* Good, because one command, not two, goes on from a child's history.
* Bad, because a child's history cannot be continued in two places at once.
