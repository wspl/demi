---
status: proposed
---

# A waiting message holds its controls

## Problem

While the agent works, the user sends "Also add a case for the expired
cookie." It waits as a steer or in the queue. Today its bubble looks like a
sent message's with dimmer text, and Send and Recall stand outside it, in
small text under its edge, where they read as part of the next row rather
than as this message's. A queued message says nothing of itself.

## Decision

A message that waits to be sent, a steer or a queued one, stands back from
the sent ones: its words are dimmer, and it stands on half a bubble's fill,
halfway between the transcript's surface and a sent bubble's, a steer's
tinted with the accent. Its last row holds its controls, as a long
message's Show More does, in the same ghost style and in line with the
text's start: Send, then Recall, and at the row's end what happens to the
message, in faint text, a clock in the accent's color with "Sending soon"
or a queue mark with "Queued".

```text
                    ┌──────────────────────────────────────────┐
                    │ The login test is failing after the ...  │  sent
                    └──────────────────────────────────────────┘
                              ┌────────────────────────────────┐
                              │ Do not touch notes.txt.        │  steer, tinted
                              │ Send  Recall    (c) Sending soon│
                              └────────────────────────────────┘
                    ┌──────────────────────────────────────────┐
                    │ Also add a case for the expired cookie.  │  queued
                    │ Send   Recall                   = Queued │
                    └──────────────────────────────────────────┘
```

A long waiting message keeps Show More first in that row, a divider, then
Send and Recall. On a phone Send and Recall are a finger's height, and the
two wrap together. A message's text keeps to its bubble's end, waiting or
sent, so when a row of controls makes the bubble wider than a narrow text,
the bubble that follows, as wide as the text or its Show More, ends where
it did: the text does not move as the message is sent.

`UserBlock` draws it from its `pending` prop as before. The steer's status
was "Sends at the agent's next step", which the row has no room for; it
reads "Sending soon" now, and the new "Queued" says the queue's.

## Spike

None.

## Prior art

- Codex's terminal UI lists the messages waiting while a turn runs under the
  transcript
  ([`pending_input_preview.rs`](https://github.com/openai/codex/blob/67f0994b532b8740f14b5b738af7b7b2a915b1d6/codex-rs/tui/src/bottom_pane/pending_input_preview.rs)):
  pending steers first, each saying it is submitted after the next tool
  result unless the interrupt key sends it now, then the queued messages,
  with a key that takes them back into the composer. Demi shows the same two
  states and the same two actions, on each message where it stands in the
  transcript.
- Messages on macOS and iOS marks an outgoing message it has not delivered
  beside the bubble, with its status under it, and keeps it in its place in
  the conversation. Demi departs in putting the controls inside the bubble,
  where a long message's Show More already is.

## Alternatives

- A draft's dashed outline and no fill: tried on the product, it read as
  something other than a message.
- A status line at the bubble's top: the line goes as the message is sent,
  and the words move up with it.
- Icon buttons at the text's end: they need a hover to be read.
- Full-strength fills and words: the waiting messages drew the eye before
  the sent ones.
- A card with a divider and a filled Send: the waiting message outweighs
  the sent ones around it.
- A badge on the bubble's corner over dimmer text: the shape still reads as
  sent.

## Open questions

None.
