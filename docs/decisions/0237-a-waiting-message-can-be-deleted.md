---
status: proposed
---

# A waiting message can be deleted

## Problem

While the agent works, the user sends "Also rename the helper." It waits
as a steer or in the queue, and then the agent renames the helper on its
own. The message is no longer wanted. Its controls are Send and Recall,
and Recall puts its words back in the composer, or, when the composer
holds a draft, keeps them for a toast's Undo. To be rid of it, the user
recalls it and then clears the composer by hand, minding not to clear a
draft they were writing.

## Decision

A waiting message, a steer or a queued one, has Delete beside Send and
Recall, in its row of controls and in its menu. Delete takes the message
out of the conversation, with its files and its comments, and gives
nothing back: the composer and its draft stay as they are.

```text
                    ┌──────────────────────────────────────────┐
                    │ Also rename the helper.                  │  queued
                    │ Send  Recall  Delete            = Queued │
                    └──────────────────────────────────────────┘
```

- The three are the same ghost buttons in the same order everywhere:
  Send, Recall, Delete, as a menu puts the destructive item last. On a
  phone each is a finger's height and the three wrap together.
- Delete acts at once, with no confirmation and no Undo: the user wrote the
  message moments ago, and Recall is there to keep its words. A message
  sent from the saved list that turns out not delivered goes from the list
  as Delete on it does today, with the list's Undo.
- Delete makes the same removal Recall makes (`removeWaiting`: the lane's
  `cancel_pending_steer` or `dequeue_message`); only what follows
  differs, so the protocol does not change. The session's log calls the
  removal `input.withdrawn`, which Recall, Delete and Clear make alike.
- Pressed on a message still on its way, Delete is kept as the user's
  wish, as Recall and Send are, and carried out once the session holds
  the message; a message that turns out not delivered is simply dropped.
- When the agent read the message first, the removal is refused: the
  message shows where the agent took it, with no toast, as after Recall.
  Any other failure brings the row back, with a toast that says the
  message was not deleted.

Delete is one action on every message the agent has not read, not
delivered, on its way, a steer or a queued one, so it is one event and one
store function: the message list's `deleteSubmission`, which a message not
delivered emitted, becomes `deleteUnsent`, and the store's `deleteSend`
becomes `deleteUnsent(conversation, id)`, which tells the four apart. The
wish kept for a message on its way becomes `'recall' | 'delete' |
'sendNow'`. `UserBlock` emits the `delete` it already has, now for a
waiting message too; the gallery's flows delete as the store does.

## Spike

None.

## Prior art

- Replit Agent's queue offers, on each queued message, Steer now, Edit
  and Delete
  ([Steer and queue messages](https://docs.replit.com/features/agent/steer-and-queue-messages)).
- Zed's agent panel gives each queued message a Remove button, "Remove
  Message from Queue"
  ([#46954](https://github.com/zed-industries/zed/pull/46954)), beside
  editing it ([#47234](https://github.com/zed-industries/zed/pull/47234)).
- Codex's terminal UI only takes a queued message back into the composer,
  as Recall does; Demi keeps that and adds what Replit and Zed have.

## Alternatives

- Undo in a toast after Delete: it would differ from Delete on a message
  not delivered, and Recall already keeps the words for a user who may
  want them.
- A confirmation: it asks for two presses for a message seconds old.
- A trash icon in place of the word: the row's other controls are words,
  and an icon alone needs a hover to be read.

## Open questions

None.
