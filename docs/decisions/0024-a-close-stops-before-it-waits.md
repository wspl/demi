---
status: accepted
---

# A close stops what it owns before it waits

## Context and Problem Statement

For example, a deploy stops the backend while a conversation's main agent
waits for a model answer that never ends, and the agent has just asked to
start a subagent on the same provider. The start waits until the main agent
gives its provider runtime back. The shutdown disposes the conversation's
tree, and the tree first waits for the starts under way, then stops its
sessions. So the shutdown waits for the start, the start for the main agent's
turn, and the turn for the stop that comes after the wait. The backend never
finishes stopping: every request answers 503, the new release cannot start,
and another subagent of the conversation keeps calling the model, since its
stop comes later still. The service unit has no stop timeout, as a shutdown
saves every Cloud, so only a kill ends it.

The same order runs through the whole close:

- A tree waits for its starts and closes before it stops any session.
- The agent server disposes one tree after another, each under its opening
  order, so a slow tree leaves the agents of the others running.
- The shard pool closes one shard after another, so one shard that hangs
  keeps every later shard open.
- Nothing tells a session that a close began until its own dispose, late in
  that order, and the close's steps log nothing, so a journal cannot tell
  which one waits.

## Considered Options

* A close stops everything it owns at once, without waiting, and only then
  waits; each wait depends only on work already told to stop.
* Keep the order, and give the service unit a stop timeout.
* Bound each wait of the close with a deadline.

## Decision Outcome

Chosen option: a close stops before it waits, because then no wait of the
close can depend on work that only the close would end. A stop timeout kills
every deploy that meets the case, losing the final saves without a trace,
and a deadline per wait guesses how long a save may take.

* A session can be stopped at once: it refuses new work, its running action
  is cancelled as by a shutdown, and a start that waits for its runtime
  learns it is closing and fails. Its dispose stops it first, then saves its
  final checkpoint.
* A tree's dispose stops every session it holds, the main agent, every
  subagent and every side question, before it waits for the starts and
  closes under way.
* The agent server's shutdown stops every live tree before it disposes any.
* The shard pool tells every shard to close, then waits for them all.
* Each awaited step of a shard's close logs when it ends and how long it took,
  so the journal of a close that hangs names the step after the last one
  logged.

### Consequences

* Good, because a shutdown ends however a turn, a start or a close stands,
  and no agent runs on while the close waits.
* Good, because a hang in a later release shows in the journal by its step.
* Bad, because a start that was about to succeed when the close began fails
  instead; the next start of the backend resumes the turns the close marked
  as interrupted, as before.
