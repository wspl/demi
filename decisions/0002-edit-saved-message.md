---
status: accepted
---

# A saved message is edited in place, in the composer's edit mode

## Context and Problem Statement

Edit on a message of the Saved list moves the message out of the list and
into the composer as its draft. A draft the composer held goes into the list
as a new saved message. Saving the edited message puts it back at the top of
the list as a new one. The user sees the message taken out and pushed back,
loses its place in the list, and finds the draft they were writing turned
into a saved message. Nothing in the composer says that a saved message is
being edited, and Escape does not leave the edit.

This record covers editing a saved message. It also adds the line that says
what is being edited to the edit of a sent message, so the two edits look
the same. It leaves out the Saved list's other actions.

## Considered Options

* Edit a saved message in the composer's edit mode, the one a sent message
  is edited in, with the message kept in the list and the draft kept aside.
* Keep moving the message into the draft, and only add a line and an Escape
  that moves it back.
* Edit the message inside its row of the list.

## Decision Outcome

Chosen option: the composer's edit mode, because it already edits one message
apart from the draft, with Escape and a Cancel button, and a saved message
needs the same.

* Edit opens the message in the composer's edit mode and closes the list. The
  message stays in the list, in its place, and the Saved count does not
  change. The draft stays the draft, out of sight, and comes back unchanged
  when the edit ends.
* A line at the top of the composer says what is being edited: "Editing
  saved message", and "Editing message" for a sent message's edit.
* The composer's end shows three buttons:
  * Cancel (×), in the place of Save for Later, which leaves the edit with the
    message unchanged, as Escape does.
  * Save, drawn as a floppy disk, which writes the edit back to the same
    message in its place and leaves the edit.
  * Send, which sends the edited message as the composer would send it, and
    the message leaves the list. Return sends, as it does in the composer.
* The server writes a saved message's new content in its place. Sending an
  edited saved message sends its new content and removes it from the list in
  one step. Moving a saved message into the draft goes away.

```text
┌──────────────────────────────────────────────────┐
│ Editing saved message                            │
│ Fix the login test, then rerun the suite         │
│                                                  │
│                                 (×)  (💾)  ( ↑ ) │
└──────────────────────────────────────────────────┘
  ✎ 3 Saved ⌄
```

### Consequences

* Good, because a saved message keeps its place, the draft is never turned
  into a saved message, and Escape leaves the edit as it leaves a sent
  message's edit.
* Good, because the two edits share one mode and one look.
* Bad, because the server gains a write of a saved message's content and an
  edited send, and loses the move into the draft.
* Modules touched: the saved messages' routes and storage in the backend, the
  web API protocol, the page's saved messages and edit state in `web`, and
  the composer and its edit mode in `web-ui`. No data to migrate: saved
  messages keep their shape.
