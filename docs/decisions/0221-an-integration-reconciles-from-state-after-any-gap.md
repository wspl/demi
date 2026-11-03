---
status: proposed
---

# An integration reconciles from state after any gap

## Problem

A team's chat bot posts each turn's answer to its group and marks the
person's message done. The bot's program restarts for a release while two
turns run; one of them ends during the 20 seconds the program is away, and
another, started by a background command's end, both starts and ends then.
The socket only sends what happens while it is connected, so the program
never learns of either: the group gets no answer. A program that reads
slowly loses more: at 4,096 unread events Demi ends its socket, and with it
every command request that socket was serving.

The program cannot rebuild what it missed either: a conversation shows its
status, but not which turn ended, how, or with what answer.

## Decision

The state is what the program trusts; the socket's events only tell it
when to look. This is how the web app stays right: a page that reconnects
lists the conversations again and reads each transcript from the version it
holds, and nothing it missed matters.

### What a conversation shows

`Conversation` gains what the sidebar's entry and the conversation's
metadata already read from its checkpoint, without loading its transcript
(`conversation/summary.rs`):

```rust
pub struct Conversation {
    // id, key, cloud, title, status, createdAt, updatedAt as today
    /// The main agent's output revision, which every change of its
    /// transcript raises.
    pub revision: u64,
    /// The main agent's latest ended turn; null before one ended.
    pub last_turn: Option<LastTurn>,
}

pub struct LastTurn {
    /// The turn's id: the program's message id for a turn its message or
    /// its steer to an idle agent started, Demi's own for one a schedule, a
    /// wakeup or a subagent's message started. Null, with `cause`, where
    /// the transcript holds no input of the turn.
    pub id: Option<TurnId>,
    /// What started it, from the block it starts with: `message` (a user
    /// block), `schedule` (a user block a schedule's run wrote), `wakeup`
    /// or `agent_message`. A resume goes on with the turn it resumes.
    pub cause: Option<TurnCause>,
    /// `finished`, `failed` or `stopped`.
    pub outcome: TurnOutcome,
    /// The block that ended it: a turn a resume continues ends anew.
    pub ended_by: BlockId,
    /// The Markdown of its last text that is not blank; null for none.
    pub answer: Option<String>,
}
```

`status` already says whether more is coming: it stays `running` while a
subagent is live, a command that is not a service runs, or a report waits
for the main agent, since each of them wakes it (`Tree::is_running`). A
turn that ended with work still out shows its `lastTurn` while the status
is still `running`; `completed`, `error` and `stopped` mean nothing will
start by itself.

### How a program reconciles

After it connects, and whenever it is told it fell behind, the program
lists its conversations (`GET /v1/conversations`, 0220's pages) and compares
each one's `revision` and `lastTurn` with what it handled. A turn it never
saw start is there as well as one it saw. A transcript it follows it reads
from the version it holds, as today.

### The socket

* The `conversation` event carries the whole `Conversation`, the shape the
  list answers, at each change of its status, `lastTurn` or title; the
  `transcript` and `agent` events stay as they are. It is read where the
  page's sidebar learns of a change, at each mark of the conversation's
  entry or metadata, and changes that come while a read waits are in it.
* A program that falls behind no longer loses its socket. When the events
  it has not read reach the bound, Demi drops the ones after and sends
  `behind` once the program reads again: its next step is a reconciliation.
  The bound stays, for a program that stops reading would otherwise hold
  the backend's memory without end.
* The plugin's traffic, its requests, their cancellations, the answers to
  their port messages and what a `declare` became, never waits behind
  events nor is dropped: it goes on a queue of its own, which the socket
  writes first. Its length is bounded by the calls the integration's agents
  make at once.

### Stored and spoken

Nothing new is stored: each field is read from the checkpoint the sidebar
reads. No older program exists outside this branch.

## Spike

None. `SummaryFacts` (`backend-database/src/tree.rs`) already holds the
latest ended turn's terminal block, outcome and answer, and its walk back
through the transcript reads every block's kind and turn, so it can go on
to the turn's first block and keep its id and kind as well. That costs a
walk over the latest turn's rows, their texts not decoded, where today it
stops at the turn's last input; only an integration's conversation reads
on, and the sidebar's summary stops where it stopped. `Tree::is_running`
already counts the work still out.

## Prior art

* Kubernetes' list and watch: a client lists, watches from the list's
  resource version, and lists again when the watch says it is too old; the
  state, not the stream, is the truth
  ([efficient detection of changes](https://kubernetes.io/docs/reference/using-api/api-concepts/#efficient-detection-of-changes)).
* GitHub keeps each webhook delivery and lets a receiver redeliver it, an
  event log that this record declines.

## Alternatives

* **A durable event log with sequence numbers and resumption**, as the
  integration that asked proposed. It stores a second copy of what the
  transcripts and checkpoints hold, needs a retention to choose, and a
  program that resumes past it must reconcile from state anyway.
* **Separate `turn_started` and `turn_ended` events.** The transcript's
  patches already carry each turn's first block and its terminal block, and
  `lastTurn` says the rest.

## Open questions

None.
