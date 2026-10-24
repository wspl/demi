---
status: proposed
---

# A message the user sends is answered or given back

## Problem

Alex asks the agent to compact the conversation, and while it summarizes,
sends "also check the logs". The page shows the message delivered. When the
summary is done, the message shows under it, and nothing answers it. Three
minutes later Alex sends it again.

A message sent while the agent works goes one of two ways: Steer, which
reaches the running turn, or Queue, which waits for the turn to end. The
session accepts a steer whatever action runs, and each action decides for
itself what becomes of the steers that wait:

| What runs when a steer waits | What becomes of the steer                     |
| ---------------------------- | --------------------------------------------- |
| A model's turn               | Read at its next boundary, answered           |
| A compaction                 | Written after the summary, unanswered         |
| A switch of versions         | Dropped when the switch ends                  |
| The user's Stop              | Written into the stopped turn, unanswered     |
| A turn that fails            | Written into the failed turn                  |
| A backend that shuts down    | Written into the interrupted turn             |
| Nothing                      | Refused; the page sends it as a new message   |

An action that ends drops the steers still waiting, with no line in the log
and no word to the page. Agent messages and command reports, by contrast,
never wait unread: when the session is idle, its scheduler opens a turn for
them. Only what the user sends can be lost, and whether it is depends on
which action happened to run.

## Decision

**What the user sends leaves the waiting list in one of two ways only:** the
model reads it in a turn that goes on to answer it, or the user's Recall
gives it back to the composer. It is never dropped, and never written into
the transcript where no turn answers it.

**Steer means as soon as possible, and Queue means after this turn:**

* A steer joins the running model's turn at its next boundary, as it does
  today.
* A steer that no model's turn takes joins the next turn: when the running
  action ends without a boundary, as a compaction or a switch of versions
  does; when a turn fails, is stopped or ends meanwhile; and when nothing
  runs. When a queued message is next, as one the user sent now, the steers
  join its turn at its first boundary; otherwise the first steer opens the
  turn, ahead of the queue, and the others join it.
* A queued message runs once the running action ends, in order, as it does
  today.

**Stop stops the work, not what the user sent.** The steers that wait when
the user presses Stop join the next turn, which starts at once, as a
correction the user sent the stopped work. A message whose turn had not
written it yet when the user pressed Stop is written with the stop, as a
moment later, so it shows with the Stopped marker. Recall on a waiting
message gives it back to the composer, as it does today.

**The session's scheduler is the one owner of the rule.** No action but a
model's turn takes the user's input, and only at its boundaries. When an
action ends, the scheduler starts the next one, and a steer that waits comes
before the queue.

The design to build from:

* `agent-session`:
  * `SessionCore::end_action` no longer drops the steers that wait.
  * `SessionCore::start_next` opens the next turn with the first steer that
    waits, as `ActionKind::Send` with its id, content and comments, unless a
    queued message is next. The turn's first boundary writes the other
    steers.
  * A queued message sent now ends the running turn with the steers left
    waiting, so they join the message's turn.
  * `SessionCore::steer` accepts a steer whatever runs, while the session is
    open: `SteerError::{NotRunning, Finishing, Stopped, Editing}` go. On an
    idle session the steer starts its turn at once.
  * The compaction and the switch of versions take no input. The
    compaction's own write of the input, and its turn for agent messages, go:
    what waits when it ends goes to the scheduler, which opens a
    continuation for agent messages and command reports as it does after any
    action.
  * `SessionCore::record_stop` and a failed turn write the agent messages
    and the command reports that wait, as now, and no steer. A steer is the
    user's own act, which ends a failed turn's hold, as a message sent then
    does. The user's Stop writes the message its turn had not written yet;
    a hold, as a Cloud reset, puts it back at the front of the queue.
  * A session that closes keeps the steers that wait at the front of the
    checkpoint's queue, as messages, whatever ran, instead of writing them
    into the interrupted turn.
* Protocol: `steer_result` is no longer `rejected` for what runs.
* `web-ui`: `ConversationRuntime::steer` no longer sends a refused steer
  again as a message. The page shows a steer that opens a turn as it shows
  a message that starts one, since the session lists it by the same id.

A page of the previous release, open while the backend upgrades, works as
before: its fallback for a refused steer is never needed.

## Spike

* **Does a switch of versions drop a steer?** A probe test admitted a
  switch and a steer in one poll. The steer was accepted, and after the
  switch it was neither waiting nor in the transcript, and no request
  carried it.
* **Can a turn's end drop a steer that arrives just before it?** No. The
  turn's last look at what arrived and the action's end run with no wait
  between them, and the session runs on one thread, so no steer is admitted
  in between.
* **What does the page do with a refused steer?** It sends it again as a
  message with the same id (`ConversationRuntime::steer`), so a steer to an
  idle session is answered today, by the page's rule rather than the
  session's.

## Prior art

* **Codex CLI**
  ([`turn_input.rs`](https://github.com/openai/codex/blob/970111f6cb62cc02fe36a82ea7d71b37a2889c97/codex-rs/core/src/session/turn_input.rs#L374-L393)):
  Enter steers and Tab queues. The core takes a steer before the turn's next
  request and starts a turn only when none runs. It refuses a steer into a
  compaction, and the terminal UI holds it and sends it first once the
  compaction ends. On Esc with steers waiting, it interrupts and sends them
  as a new turn
  ([`interaction.rs`](https://github.com/openai/codex/blob/970111f6cb62cc02fe36a82ea7d71b37a2889c97/codex-rs/tui/src/chatwidget/interaction.rs#L272-L284)).
  Input left when a turn finishes is written to the history unanswered
  ([`tasks/mod.rs`](https://github.com/openai/codex/blob/970111f6cb62cc02fe36a82ea7d71b37a2889c97/codex-rs/core/src/tasks/mod.rs#L700-L724)),
  which this decision does not do.
* **OpenCode**
  ([`inbox.ts`](https://github.com/sst/opencode/blob/6806c6e29a7d2087335794d8b86c6031cdb5f6dd/packages/core/src/session/inbox.ts#L505-L541)):
  one durable inbox the core owns, each item marked steer or queue. Steers
  go first, at the next step boundary; queued items one at a time when the
  session is idle. A late arrival rings a doorbell that drains the inbox
  again. On interrupt, the steers resume in a new run
  ([`execution.ts`](https://github.com/sst/opencode/blob/6806c6e29a7d2087335794d8b86c6031cdb5f6dd/packages/core/src/session/execution.ts#L148-L163)).
* **Gemini CLI**
  ([`useMessageQueue.ts`](https://github.com/google-gemini/gemini-cli/blob/cd1b45c2db0a5ea7b7fc3b67c1ba5879601d6005/packages/cli/src/ui/hooks/useMessageQueue.ts#L70-L91)):
  the terminal UI queues what is sent while the model works or a
  compression runs, and sends it as one prompt once idle. Steering is an
  experimental setting; a steer no tool result carries is sent as a new
  prompt once idle.
* **Claude Code**, by its documentation only
  ([interactive mode](https://code.claude.com/docs/en/interactive-mode#queue-messages-while-claude-works)):
  a message sent while Claude works is read as soon as the running tools
  finish, and what is left when the turn ends is sent in order. Esc
  interrupts and sends the waiting messages at once; Up takes them back into
  the input.

All four keep what the user sent while the agent worked, and three of the
four send it on after an interrupt. Demi takes OpenCode's shape, one owner
in the session, with Codex's and Claude Code's interrupt.

## Alternatives

* **Answer the steers after a compaction, and fix each action alike.** That
  fixes the case at hand and leaves the rule in every action, where the next
  action can miss it, as the compaction and the switch did.
* **Stop gives the waiting steers back to the composer.** The user then
  sends them again by hand, while a steer sent during the work is most
  often the correction the Stop makes room for; Recall gives back one the
  user no longer wants.
* **One list for steers and queued messages, as OpenCode keeps.** A cleaner
  model, but the protocol's two lists and the page's two kinds of rows would
  change with it, and nothing in this decision needs it.

## Open questions

None.
