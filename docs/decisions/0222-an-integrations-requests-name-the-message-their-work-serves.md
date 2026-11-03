---
status: proposed
---

# An integration's requests name the message their work serves

## Problem

In a team's group chat, alex asks the bot to deploy, and the bot's
`chatbot deploy` command must check that alex, not whoever spoke last, may
deploy. The agent spawns a subagent for the build, whose command runs after
sam has asked something else; later a background command ends and wakes
the agent, which runs the command again. The command request names the
conversation, the Cloud and the agent's number, but not the turn: the
program cannot tell which person's message the call serves. A context
request, which tells the agent who is speaking, cannot tell either, and when
the program is slow it is skipped after 10 seconds, so the agent runs
without knowing who speaks. The usage the program pays per person cannot be
split by message. And a program that names its conversations itself cannot
stop the title request each one spends on the user's small model.

## Decision

Demi keeps no data of the program's (0149): the program's message id, which
it chose, is the key it finds its own data by. Each request names the
message the work descends from.

### What a request names

`PluginRequest::Command` and `PluginRequest::Context` gain:

```rust
/// The turn the agent works in: its own id, as `lastTurn.id` (0221).
pub turn: String,
/// The id of the program's message this work descends from; null when it
/// descends from none, as a schedule's turn does.
pub message_id: Option<String>,
```

A command request also gains `agent: Option<NodeId>`, the agent's node,
which a context request names already. A command's agent and turn are null
for a job no agent's call started.

A turn's message is read from the transcript, not stored:

* a turn the program's message started, or its steer into an idle agent:
  that message's id;
* a turn a wakeup started: the message of the turn whose call started the
  command of its first report;
* a turn of a subagent: the message of its parent's turn that spawned it,
  whose call the subagent's record names (`nodes.started_by`); a subagent
  resumed in a later turn keeps it, since its work goes on from its spawn;
* a turn a subagent's message started in its parent: the message of that
  subagent;
* a schedule's turn: none.

A turn the program's message did not start but steered keeps its message:
the steer is the program's to read in the transcript.

A turn's first block is saved a moment after it is written, and its first
request asks for context before then, so the agent's rows are saved before
the message is read. The rows of the turns it descends from were saved before
their calls ran.

### Usage

Each ledger row already keeps its turn (`usage_ledger.turn_id`). A row of
an integration's conversation keeps the message as well, as it was when
written, as it keeps its device and integration (0156);
`GET /v1/usage?by=message` groups by it, and `by=turn` by the turn.

### A context the agent cannot go without

`Declaration.context`, the flag 0152 declared, becomes `none`, `optional`
or `required`; a declaration stored with the flag reads `optional` for
`true`:

* `optional` is today's: a request unanswered within the wait is skipped,
  for context that only helps.
* `required` waits for a socket as a command request does, for as long as
  the backend waits for a runner that dropped, and for its answer as long
  as an optional one; when the program does not answer, the agent's request
  fails before any vendor, as one whose system prompt
  cannot be made does, and the turn ends with an error the program sees in
  `lastTurn` (`context_unavailable`). No request goes to the model without
  it.
* The wait is the declaration's `contextWaitSeconds`, 10 when it names
  none, since a program that knows how slow its context source is sets the
  wait, not Demi.

### Titles as a capability

An integration gains the capability `titles`, on by default as every
capability is. While it is on, a conversation is titled as the web app's
are: the start of its first message at once, then the title the user's
small model writes. While it is off, the conversation makes no title
request, and its title stays the start of its first message. Either way,
the program may name it with `PATCH {title}`, which a title request that
ends afterwards does not overwrite.

## Spike

* The ledger's `turn_id` column exists today (`schema/control.sql`).
* A shell call's row names its command (`blocks.command`), and the input
  blocks of a turn carry its id (`blocks.turn_id`), so the turn of any row is
  that of the nearest row before it on its path that carries one, and every
  rule above reads stored rows.

## Prior art

* Slack and Telegram bot APIs hand the bot the id of the message an event
  is about and keep nothing of the bot's; the bot looks its own records up
  by it.
* OpenTelemetry's trace context carries a trace id from parent to child
  work, so each span names the request it serves
  ([W3C Trace Context](https://www.w3.org/TR/trace-context/)): `messageId` is
  that trace id.

## Alternatives

* **Opaque metadata on each message**, echoed in requests, as the
  integration that asked proposed. Demi would keep a second copy of the
  program's data, bounded by a size limit it would have to pick.
* **`origin` as the field's name.** Agent-server's `Origin` already names
  how a node came to exist.
* **The turn alone.** A subagent's or a wakeup's turn is Demi's own, so the
  program would have to walk the transcript to find the message.

## Open questions

None.
