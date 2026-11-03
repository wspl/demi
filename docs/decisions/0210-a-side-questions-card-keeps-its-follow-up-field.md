---
status: proposed
---

# A side question's card keeps its follow-up field

Supersedes: 0056 (A side question can be stopped), the part that moves the
focus to the card's Reply after Stop: it moves to the follow-up field, or to
the card while it is folded.

## Problem

A user asks "Why is the timeout the same for uploads?" about a reply and
reads the answer. To ask more, they first find and press Reply, a small
button under the answer, which then turns into a field. The card grows by
the field's height as it opens, so the answer above it moves, and an empty
field closes again when a click lands outside the card. While the question
works there is no Reply at all, so a follow-up the user has in mind waits
until the answer ends, and the button appears then, moving what is under
the card.

The field, once open, starts its text 4px from its round end, so the
placeholder sits in the curve.

## Decision

An open card ends with its follow-up field in every state: while the
question works, once it is answered, stopped or failed. There is no Reply
button.

```text
╭──────────────────────────────────────────────────────────╮
│ (?) Why uploads time out                         ↗   ⌃   │
│ Why is the timeout the same for uploads?                 │
│ All API calls share one client, created in client.ts …   │
│ ╭──────────────────────────────────────────────────────╮ │
│ │  Ask a follow-up                                (↑)  │ │
│ ╰──────────────────────────────────────────────────────╯ │
╰──────────────────────────────────────────────────────────╯
```

* While the question works, the field takes what is typed; its send button
  stays dimmed and Return sends nothing, until the answer ends and the user
  sends again. Stop stays in the header.
* A failed answer keeps Retry above the field, and the field sends a
  follow-up as after an answer.
* Nothing closes the field. Escape does nothing in it, as in the composer.
* The field never takes the focus by itself, so a card that opens does not
  take it from the composer. After Stop, the focus moves to the field, or to
  the card while it is folded.
* A folded card shows its header alone, as before. What the field holds
  stays with its card while the card is folded or out of view.
* The text starts 12px from the field's round end.

`AskCard` loses its `replying` model; `MessageSendField` takes `canSend`,
false while the question works, and loses `focus` and `cancel`. Nothing is
stored and no protocol changes.

## Spike

None.

## Prior art

GitHub's pull request review threads and Notion's comment threads keep a
reply field under each thread, which reads as one line until it is used.
ChatGPT's and Claude's composers stay in place while the model answers and
take what is typed; they send nothing until the answer ends or is stopped.

## Alternatives

* Keep Reply and reserve the field's room under it: the card would hold an
  empty line that only a press fills, and a follow-up still costs a press.
* Send a follow-up typed while the question works into its running turn: the
  card shows one answer per question, and a question that joins a running
  answer would have none of its own.

## Open questions

None.
