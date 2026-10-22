---
status: superseded by 0052
---

# A command that ends reports until it ends

Supersedes, in part: 0015 (a command reports itself), where `--resident`
stays.

## Context and Problem Statement

An agent runs a test suite as one command that reports every ten minutes.
A test hangs. The first report says the command has printed nothing for
seven minutes; the agent answers it with `demi shell status --resident`,
which the report's own last line offered, and ends its turn. The command
never ends, so nothing wakes the agent again, and it waits until someone
tells it.

`--resident` turned a command that ends into one that reports only its
end. For such a command, its end is exactly what a hang withholds. The
shell rules of the system prompt taught how to wait, which flag to use and
not to poll, but not that a running command is the agent's to watch.

## Considered Options

* Keep `--resident`, and stop the reports from offering it.
* Remove `--resident`, and say in the rules what a command asks of the
  agent instead of how to wait.

## Decision Outcome

Chosen option: remove it, because a command that ends then always reports
while it runs, and a report that shows a command has stopped making progress
always reaches the agent.

* `demi shell status` changes how often a command reports with `--interval`
  alone. A command started with `intervalMs` null still reports only its
  end, as before.
* A report of a running command says how often it reports, and no more.
* The shell rules say what the tool does and that a command the agent
  starts is its own to watch until it ends; how to wait, poll or sleep is
  left to the agent and to each command's help.

### Consequences

* Good, because a hung command keeps reporting, its idle time growing,
  until the agent looks into it.
* Bad, because an agent that waits on a long command hears from it every
  interval, at most every ten minutes, where it could ask for silence.
