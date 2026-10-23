---
status: proposed
---

# A running command keeps its conversation running, and services count apart

## Problem

An agent starts a build of ten minutes, which goes to the background after
30 seconds, and ends its turn to wait. The sidebar shows the conversation
with a green dot, finished, while the build still runs and the agent will
go on when it ends. Above the composer, one chip says "2 Running" for the
build and the development server the agent started an hour ago, which
runs until stopped: the user cannot tell work that will end from a server
that stays.

## Decision

**A command that is not a service keeps its conversation running.** While
a command an agent started without `service: true` still runs after its
turn ended, the conversation's status is running, as when a subagent still
works: its dot is the accent, breathing, in the sidebar, the tabs and the
agent's mark. A service does not: a conversation whose only running
command is a development server is finished, green until read.

**Two chips above the composer:**

```text
  ┌──────────────────┐ ┌────────────────┐
  │ ● ▣ 1 Running    │ │ ∿ 1 Service    │
  └──────────────────┘ └────────────────┘
```

* **N Running** counts the commands that went to the background and are
  not services, with the breathing dot and the terminal icon it has today.
  A command whose call still waits for it is not counted: the chat shows
  it running.
* **N Service** or **N Services** counts the services, with the Activity
  icon, a pulse line, and no dot: a service that runs is in its settled
  state.
* Each chip shows only while its count is above zero. Pressing it opens
  the terminal window on its oldest command; while the window shows a
  command of the other chip, pressing it moves the window to its own,
  as pressing Agents while Running is open does; pressing it again closes
  the window.

What follows for the user, by the same rule:

* The notification that a conversation finished comes when the agent's
  turn after the command's end finishes, not when it ended its turn to
  wait.
* Archive is unavailable while such a command runs, as while a subagent
  works.
* Stop leaves the commands that went to the background running, so the
  conversation stays running until they end.
* A command that never ends and was started without `service: true`,
  such as `tail -f`, keeps the conversation running until it is stopped.

The design to build from:

* The backend's conversation status, `Tree::is_running` in agent-server,
  counts a running command that is not a service, where it counted none.
  The sidebar summary, the turn-end notification and the status report
  read it. The page counts by the same rule (`isConversationActive`) for
  the conversation it shows.
* Whether a command is a service is its `service` input, which the backend
  keeps with each running command (`running_commands.service`). The
  command's view for the page carries it (`CommandView.service: boolean`),
  and the page's `TerminalRecord` holds it.
* One chip component draws both chips; which commands it counts, its icon,
  its dot and its words are its inputs.
* The gallery's chip specimens show both chips, alone and together.
* An open page from before ignores the field `service`, since the page's
  decoder keeps unknown fields out, and counts every command in its one
  Running chip until it reloads.

## Spike

None.

## Prior art

* Codex's terminal interface counts every background command in one line,
  "{count} background terminals running · /ps to view · /stop to close"
  ([unified_exec_footer.rs](https://github.com/openai/codex/blob/main/codex-rs/tui/src/bottom_pane/unified_exec_footer.rs)).
  Demi counts services apart, because a server that runs for hours would
  keep that count above zero, and keeps the conversation running, because
  the command's end wakes the agent.

## Alternatives

* **One chip, as today.** A development server keeps the count above zero
  for hours, so the chip no longer says that work is in progress.

## Open questions

None.
