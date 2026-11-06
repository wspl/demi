---
status: accepted
---

# The last message sent mid-turn can be edited

Supersedes: 0167 (A message being answered can be edited), the part that
offers Edit only on a message that starts a turn.

## Problem

Alex reads the agent's answer and at once sends "Include the other repository
too." The answer's last words were already on screen, but the turn had not
closed yet, so the message joined the turn as a steer. The agent thinks for
two seconds and Alex presses Stop. The message shows Copy and no Edit, and
the Edit that is offered sits on Alex's message before it.

Whether a message starts a turn or joins the one running depends on a few
milliseconds Alex never sees, and both look the same in the transcript. Only
the one that starts a turn can be corrected.

## Decision

**Edit is offered on the last message the user sent, whether it started the
turn or joined it.** The rules of 0167 stay: on an idle conversation, and on
the running turn's last message while nothing else waits. Up Arrow in an
empty composer opens the same message, and Regenerate, in the footer of an
answer after it, answers it again.

An edit of a message that joined a turn keeps everything before it: the
turn's answer up to that point stays. The edited message takes its place
and starts a new turn, as a message sent then would have. The old message
and what followed it are the earlier version, and the switch between
versions shows on both.

```text
  Before                          After the edit
  You: Rename the module          You: Rename the module
  Agent: Renamed it to billing.   Agent: Renamed it to billing.
  You: Also the tests     <-+     You: Also the tests and docs   < 2/2 >
  Thought for 2s            |     Agent: ...
  Stopped                   +-- version 1/2
```

A Stop still gives back only a message that started a turn with nothing of
its answer shown (0167); a message that joined a turn is not given back.

The design to build from:

- `shared-types`: `Block::is_editable` holds for a `steer` block as for a
  `user` block. `agent-transcript`'s `before_user` becomes `before_input`,
  the blocks before either.
- `agent-session`, `editing.rs`: the target is a `user` or a `steer` block.
  An edit writes a `user` block with a new turn in its place, its media and
  comments resolved against the target's own; Regenerate of a steer keeps it
  and infers again from it.
- `web-ui`: `offeredEditId` and `runningMessageId` take the last `user` or
  `steer` block; a steer's bubble has Edit and the versions switch as a
  message's does.
- The protocol does not change: an edit names its target by block ID. A
  page of an earlier release offers no Edit on a steer, as now.

## Spike

None.

## Prior art

Codex CLI's backtrack (Esc Esc) edits any earlier user message of the
transcript and forks the conversation there
(`codex-rs/tui/src/app_backtrack.rs`). Claude Code's rewind lists the user's
prompts to go back to. Neither shows the user whether a message started a
turn.

## Alternatives

- An edited steer joins the turn again, as a steer: the session would have
  to resume a turn from its middle, and the result reads the same to the
  model as a new turn after the answer so far.
- Keep Edit off steers: the user cannot correct the last thing they sent,
  for a reason they cannot see.

## Open questions

None.
