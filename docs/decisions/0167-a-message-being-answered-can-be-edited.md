---
status: accepted
---

# A message being answered can be edited, and one stopped before its answer comes back

Supersedes: 0093 (A message the user sends is answered or given back), the
part that writes a stopped turn's message into the transcript with the
Stopped marker.

## Problem

Alex sends "rename the module to billing" and sees at once that it should
have said "payments". The agent is still thinking. The message shows no Edit:
the page offers Edit only on the last message of an idle conversation. Alex
presses Stop, and the message stays in the transcript under a Stopped marker,
with nothing after it. To send the right one, Alex types it again, or waits
for the conversation to settle, opens Edit and corrects it, and the wrong
message stays behind as an earlier version.

The page hides Edit while the agent works because an edit replaces the
answer, and the session takes an edit only once nothing runs. That is how the
mechanism is built, not what Alex expects.

## Decision

**Edit shows on the message the agent is answering**, as it does once the
agent is done: on the last message, when no other message waits.

- Edit opens the message in the composer as it does now. The agent keeps
  working while Alex edits.
- Send stops the turn and continues from the edited message: the answer so
  far goes, and the old message stays as an earlier version, as with any
  edit.
- Cancel changes nothing.
- No key starts the edit; it is the message's own Edit. Up Arrow in an
  empty composer opens the editor only once the agent is done, as now, and
  Regenerate shows only then too.

**Stop before the answer starts gives the message back.** When the agent has
shown nothing of its answer yet, no thinking, text or tool, Stop takes the
message out of the transcript and gives it back to the page that stopped it
as Recall gives back a waiting message: into the composer when the composer
is empty, with its files and comments; otherwise the draft stays, and a
notice offers to send the message again. Nothing marks the stop: the
conversation is as it was before the message. Once any of the answer shows,
Stop works as now and leaves the message and the answer so far. A message a
schedule sent, a retry and a resume are not given back.

### Design

- `agent-session`:
  - An edit of the running turn's message is admitted while the turn runs:
    the session stops the turn as Stop does, then applies the edit once it
    settled, against the transcript the stop left, as one operation.
  - `SessionCore::abort_step` decides whether the Stop gives the message
    back: a send whose turn wrote only the message and the model's context,
    with no steer waiting. `SessionCore::record_stop` then writes neither the
    message nor the abort marker, takes the message back off the path when
    the turn had written it (`TranscriptLog::withdraw_from`, saved as a
    branch that stands in for it), and hands it back with the stop's result.
- Protocol: `AbortResult` gains the message given back, its content and
  comments, or none. Another page of the conversation sees the message go
  from the transcript.
- `web-ui`: `offeredEditId` offers the last message while its turn runs
  and nothing else waits; an edit opened then is sent against the
  transcript the page shows when it is sent; the page that stopped a turn
  gives the message it got back as Recall does.

A page of the previous release ignores the field it does not know: its
stopped message leaves the transcript without coming back to its composer.

## Prior art

- **Claude Code** gives a prompt back on an interrupt before any response:
  "Interrupting a prompt before any response now automatically restores your
  input so you can edit and resubmit" (its changelog). Its VS Code extension
  brings a sent prompt back into the input to edit it.
- **Codex CLI**, on an interrupt, puts the messages that wait, queued or
  steering, back in the composer, merged with the draft it held (`on_interrupted_turn`, `drain_pending_messages_for_restore` in
  `codex-rs/tui/src/chatwidget/input_restore.rs`); the prompt in flight
  stays in the history.

Where this departs: Claude Code starts the edit from a key, Escape; Demi
offers only the message's Edit, as the page does once the agent is done.

## Alternatives

- **Keep Edit for an idle conversation, and Stop as it is.** Correcting a
  message just sent takes a Stop, a wait and a retype or an edit.
- **Give back on every Stop.** Once the answer shows, Alex usually wants to
  keep what it did; the message stays as the start of that work.
- **Edit stops the turn as it opens.** Alex may cancel the edit, and the
  work would be lost for nothing.

## Open questions

None.
