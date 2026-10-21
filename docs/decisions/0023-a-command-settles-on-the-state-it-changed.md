---
status: accepted
---

# A command settles on the state it changed, which reaches the page step by step

## Context and Problem Statement

For example, while the agent works, the user sends "also check the logs". The
page shows the message on its way and sends a steer. The backend accepts it
and answers `steer_result: accepted` at once. The page drops its own row on
that answer and waits for the `pending_steers` frame to show the steer. But
the session held that frame with the end of the turn it was finishing, so the
message vanished from the page until a reload. Recall failed the same way the
other way round: the backend removed the steer and answered `removed`, and
the row stayed, since the frame without it was held.

The waiting inputs reach the page by three paths that nothing keeps in step:

- The command's answer (`steer_result`, `remove_result`), written as soon as
  the command ran.
- The state frames (`pending_steers`, `queue`), written when the session's
  event bus delivers the change, which a hold at a turn's end may delay.
- The open handshake, which reads the session's lists directly.

What lets them disagree:

- The page takes a command's answer for the state it changed. Comments say the
  answer follows the list's frame, and nothing makes it so.
- The queue and the pending steers are whole lists without a version, so the
  last frame to arrive wins and nothing notices one that is missing or late.
  [Agent sessions, and one way to keep their data in sync](0009-agent-sessions-and-sync.md)
  counts the queue among the versioned metadata; the code keeps it apart.
- A gap in the agents' version closes the socket, where 0009 asks for the
  changes since the version the page holds.
- The session holds every event at a turn's end, so that the answer's last
  text and the turn's end arrive together, and with them whatever else changed
  meanwhile.
- A message the session accepted is in no state until its turn writes it: a
  send that starts a turn shows only as the phase running until the turn has
  read what its request needs, which can take seconds when a Host wakes. A page
  that reloads then shows the message nowhere.
- A steer sent again with the same id, after an answer lost with its socket,
  is taken twice.

Telegram's update protocol ([Working with updates](https://core.telegram.org/api/updates))
solves the same problem for messages across devices. Compared item by item,
Demi already has its push delivery, its independent sequences (one per
message box, like one per agent here), its gap filling and its rule for
duplicates. It differs in three things, which this record takes:

- One `Updates` container carries every update of one event, applied together.
- A method's answer is itself updates, with the sequence number they took.
- The server takes each `random_id` once, however often a client sends it.

## Considered Options

* A command settles on the state it changed, which reaches the page as one
  frame per change of the session; an accepted input is in the state from the
  step that accepts it; an answer names the version of that step; a steer is
  taken once per id.
* Keep the answers as the page's signal, and make every answer wait for the
  state frame it follows.
* Drop the hold at a turn's end only.

## Decision Outcome

Chosen option: a command settles on the state it changed, because the page is
then right whatever order frames arrive in and however long any of them
waits. Making answers wait for their state frames keeps the order as a rule
every new command and hold must remember, and dropping the hold alone leaves
the other paths as they are.

* **The waiting inputs are versioned.** The root's queue and pending steers
  are part of the conversation's agents metadata, under its version, as 0009
  has it, and change by patches like every agent entry.
* **An accepted input is in the state from the step that accepts it.** A
  queued message is in the queue and a steer in the pending steers. A message
  whose turn starts at once is the turn's starting message in the root's
  entry, until the step that writes it into the transcript. So the state alone
  tells where a message is, across a reload too.
* **One step, one frame.** What one change of a session alters, a node's
  transcript and agent entries, reaches a page in one frame, applied together,
  as an `Updates` container is.
* **The turn's end is one step.** The answer's completed text and the phase
  that goes idle after the closing save are announced together, once the save
  has committed. Only they wait; the session holds nothing else, and the event
  bus has no hold of its own.
* **An answer names the version of its step.** A steer, a removal or a send
  is answered with the agents revision of the step that took it, or with its
  refusal. The page settles the command once it holds that revision, and
  never from the answer alone. A repeated id is answered with the revision
  that already shows it.
* **A row the page drew first stays until the state replaces it.** A message
  on its way stays on the page, by its client id, until the state shows it
  queued, pending, starting or in the transcript, or its command is refused,
  when it stays with its error and Retry. A recalled message stays hidden until
  the state no longer shows it, or comes back if the Recall is refused.
* **An input is taken once per id.** A steer whose id the session already
  holds, pending or written, is answered as accepted without a second steer,
  as a send already is.
* **A gap is filled, not reloaded.** A patch whose revision does not follow
  the one held makes the page ask for the changes since that revision, on the
  same socket, as a transcript gap does. A reset answers when the server cannot
  give the changes.
* What stays outside the version: the live output count (`request_output`)
  and a running command's output (`shell_output`), each of which carries its
  whole current value, so the latest frame is right on its own; and errors,
  which are events, not state.

```text
 user sends "also check the logs" while the agent works

 page row                         server
 ┌─────────────────────────────┐
 │ also check the logs  ◌      │  steer      ─────▶  one step: pending
 └─────────────────────────────┘                     steers [it], r41
                                  frame r41  ◀─────  agent entry
                                  answer     ◀─────  accepted at r41
 ┌─────────────────────────────┐  (settled: the page holds r41)
 │ also check the logs         │
 │ Sends at the agent's next   │
 │ step        Send   Recall   │
 └─────────────────────────────┘
                                  frame r43  ◀─────  (r42 missing)
                                  "changes since r41" ─▶  r42, r43
```

### Consequences

* Good, because a message the user sent never leaves the page before the
  server's state shows it, a Recall never leaves a removed message behind, and
  a reload shows every message the server accepted.
* Good, because a missing or late change is noticed and filled without a
  reload, and no rule depends on the order of an answer and a state frame.
* Good, because a page never applies half of one change.
* Bad, because the socket protocol changes on both sides: list frames give way
  to agent entries, frames group a step's changes, and answers carry a
  revision.
* Bad, because every change of the waiting inputs raises the conversation's
  agents revision, which every open page of the conversation applies.
