---
status: proposed
---

# Only what the user asked for notifies when it ends

## Problem

Alex gives the conversation *Watch the nightly build* a schedule that checks
the build every ten minutes. Each run is a turn, and each turn that ends
pushes a notification to Alex's laptop and phone, whether or not the run
found anything. A schedule meant to save Alex's attention takes it every
ten minutes, and the notification of a turn Alex did ask for drowns among
them.

## Decision

A turn that ends with an answer or an error notifies only when the user
asked for it: the latest message the user or a schedule wrote to the
conversation is the user's own, a message or a steer.

* A turn a schedule's message started notifies nothing, and nor does any
  later turn of that request, such as one a subagent's completion or a
  command's report woke.
* A turn of the user's request that a subagent's completion or a command's
  report woke notifies as the request's first turn would: it is the work
  the user asked for, finishing.
* A turn that a message from another conversation started
  (`demi conversation send`) notifies nothing: another agent asked for it.
* A turn of a schedule's request that the user steered notifies.
* What the agent sends with `demi notify send`, and a request that asks
  the user, notify as before, whatever started the turn. A schedule whose
  run finds something worth telling says so with `demi notify send`.

The sidebar's dots and the tab are unchanged: a schedule's turn still
marks its conversation unread.

### Design

Whether the user asked for a turn is decided as its inputs arrive, by the
transcript log, which sees every block as it is appended:

* The user's own message or steer opens or joins a turn the user asked for.
* A schedule's message, or a message from another conversation, opens one
  the user did not.
* Any other input, a command's report, a subagent's message, a resume,
  opens a turn of the work it continues: the user asked for it when the
  latest message the user or a schedule wrote, in the whole path and past
  any compaction (`Earlier::user_input`), is the user's own.

When such a turn ends with an answer or an error, the log keeps its end
with its answer until it is taken:

```rust
pub struct AskedEnd { pub id: BlockId, pub failed: bool, pub answer: Option<String> }

impl TranscriptLog {
    pub fn take_asked_end(&mut self) -> Option<AskedEnd>
}
```

The shard takes it at each rest of the tree and notifies of it at the rest
after work, unless the user stopped the last turn. A schedule's run that
waited behind the user's turn therefore never hides its notification, and
a compaction or a retry after the turn ended changes nothing. A
conversation no message of the user or a schedule ever reached has no
latest message, and its turns notify nothing. Nothing is stored and no
protocol changes.

## Prior art

* Codex CLI (`codex-rs`) runs its `notify` program at every
  `agent-turn-complete`, and has no schedule.
* ChatGPT's scheduled tasks notify of each run, since a task's result is
  what the user scheduled it for. A Demi schedule more often watches for
  something, and its agent decides whether a run found it.

## Alternatives

* **A switch per schedule, Notify when a run ends.** Every schedule would
  need it set, and the agent already judges better than a switch which
  runs matter.
* **Notify of a schedule's run only when it fails.** A failure of an
  unattended run shows on the sidebar's red dot; a push for each would
  bring the noise back on a flaky check.

## Open questions

None.
