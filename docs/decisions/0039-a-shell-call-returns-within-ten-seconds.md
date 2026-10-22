---
status: superseded by 0052
---

# A shell call returns within ten seconds

## Context and Problem Statement

A model starts a build with `intervalMs: 600000`, meaning that a running
build should report every ten minutes. The call then holds the model for 60
seconds before it returns, because a call waits for the smaller of its
interval and a minute that no tool text mentions, and an interval is at
least 15 seconds. So the wait is 15 to 60 seconds, chosen by a value meant
for something else: models that give long intervals wait a minute on every
slow command, and a response's other calls, done in seconds, wait with it.
Over four days of real conversations, the first wait of commands that ran a
minute or more took 9% of the agents' working time.

## Precedents

* Codex's `exec_command` takes `yield_time_ms`, how long the call waits
  before it returns with the command still running, 10 seconds by default
  and written in the tool's description; the command's later output is read
  apart from it.

## Decision Outcome

* A call returns when its command ends, or after 10 seconds with the
  command still running, whatever its interval, for every model.
* `intervalMs` says only how often a running command reports, as its name
  says.
* The tool's description says both.
* A resident command keeps its own rule: the call returns once its output
  has been quiet for 2 seconds, or after 30 seconds.

### Performance

* A slow command holds its call 10 seconds rather than 15 to 60; the calls
  of one response wait at most that long for one another.
* A command that ends between 10 and 60 seconds now returns first and
  reports its end, one more turn of the model.

### Consequences

* Good, because the wait is the same for every model and the tool says what
  it is.
* Bad, because a command of 10 to 60 seconds costs a report, where it
  returned in its call before.
