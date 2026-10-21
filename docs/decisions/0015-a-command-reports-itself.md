---
status: accepted
---

# A command reports itself; nothing waits for it

## Context and Problem Statement

An agent can wait for a command it started in two ways. The command reports
to the agent at its interval and when it ends, and an agent with nothing
else to do ends its turn and is woken by that report. Besides,
`demi shell status <id> --wait <duration>` blocks inside a call until the
command ends. A shell call also returns early when a message from another
agent arrives, and its result then suggests `demi shell status` as the next
step, so an agent tends to start a second call that only waits for the
first. The page then shows two running commands for one piece of work, and
the agent's instructions say both "do not poll" and how to block.

## Considered Options

* Keep one way: the command reports itself. Remove `--wait`, and no text
  the agent reads offers to look at a running command as its next step.
* Keep `--wait` and tell the agent when to use it.
* Make a shell call keep waiting when a message arrives.

## Decision Outcome

Chosen option: one way, the command's own report, because a report already
carries the command's output and wakes the agent, and one way leaves the
agent nothing to choose between.

* `demi shell status` shows a command's output and state at once and has no
  `--wait`. `--interval` and `--resident` stay: they change how the command
  reports.
* A call that returns while its command runs says that the command keeps
  running and when it reports, and how to answer its prompt or stop it. It
  does not suggest looking at it.
* The agent's instructions say to wait for a command or a subagent by doing
  other work or ending the turn, and say nothing of blocking on one.
* A message that arrives during a call still returns the call early, so the
  agent hears it within the call's window; the command keeps running and
  reports as before.

### Consequences

* Good, because an agent waits for a command one way, and the page shows
  each command once.
* Bad, because an agent that needs a command's result to go on, and has
  nothing else to do, ends its turn and is woken, which costs one more
  request than blocking did.
