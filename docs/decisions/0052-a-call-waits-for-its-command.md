---
status: accepted
---

# A shell call waits for its command, and a command wakes the agent only when it ends

Supersedes: 0015 (a command reports itself), 0039 (a shell call returns
within ten seconds) and 0048 (a command that ends reports until it ends).

## Context and Problem Statement

An agent generates an image with a command that takes about 40 seconds. The
call returns after 10 seconds with the command still running. The agent
wants the picture before it goes on, so it calls again to look at the
command, then sleeps, then looks again: a run of calls that each return
after 10 seconds, where one call would have held for 40 seconds and
returned the picture.

Two rules make this. A call returns after 10 seconds, so most commands that
take a minute outlive their call. And a running command reports at an
interval the agent must choose on every call, so an agent that waits is
woken over and over by reports that say the command still runs.

## Precedents

* Claude Code's Bash tool holds a call until its command ends, two minutes
  by default and ten at most; a command still running then moves to the
  background, and its end comes to the model as a new message, waking it
  when idle. `run_in_background` starts a command that does not end, such
  as a development server, without holding the call. A foreground `sleep`
  of 25 seconds or more is refused with a note that the end of a background
  command is announced.
* Codex returns from `exec_command` after 10 seconds by default, 30 at most,
  and never tells the model that a command ended; the model reads it again
  to learn. Its models are trained to.

## Decision Outcome

Chosen option: hold the call as Claude Code does, with a cap of 30 seconds,
because most commands an agent runs end within it and return in one call,
and an agent woken only by an end has nothing to poll.

* A call returns when its command ends, or after 30 seconds with the
  command still running in the background.
* A command in the background wakes the agent when it ends, and once when
  it has printed nothing for 10 minutes, saying so; it keeps running, and
  the agent decides whether to stop it. Nothing else wakes the agent about
  it.
* `intervalMs` is removed. The tool takes `background: true` for a command
  that runs until stopped, such as a development server or a watcher: its
  call returns once its output has been quiet for 2 seconds, or after 30
  seconds, and it wakes the agent only when it exits.
* A command that starts with a `sleep` of 30 seconds or more is refused,
  unless it runs in the background, where it is a timer; the note says that
  the agent waits by ending its turn and that a command's end wakes it.
* A message that arrives during a call still returns the call early, as
  0015 decided.
* `demi shell status` loses `--interval`.
* A report or a saved state that names an interval is read without it; no
  stored history is rewritten. Only the table of running commands changes,
  which records whether a command runs in the background in place of its
  interval.

### Consequences

* Good, because a command that ends within 30 seconds, most of them, costs
  one call and no report, and the agent chooses nothing about how it is
  told.
* Good, because a hung command still reaches the agent, after 10 minutes of
  silence, as 0048 required.
* Bad, because a slow command holds the response's other calls for up to 30
  seconds, where 0039 held them for 10.
* Bad, because a development server started without `background: true`
  holds its call for 30 seconds.

### Performance

* A command between 10 and 30 seconds returns in its call, which saves the
  model the request that read its report.
* An agent waiting on a long command makes no request until it ends or goes
  silent for 10 minutes, where it made one per interval.
