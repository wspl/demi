---
status: accepted
---

# Return in an empty composer sends the next waiting message now

## Problem

The agent is running a 40-second build. Alex types "Use the staging config
instead" and presses Return. The message waits as a steer under the turn,
saying Sending Soon, until the build's step ends. Alex decides the build
should not go on with the wrong config and wants the steer to go now. Today
that takes the mouse: Alex reaches for the bubble's Send. The same holds for
a message waiting in the queue, which waits until the turn ends.

The composer already has the focus and nothing in it, and Return there does
nothing.

## Decision

**In an empty composer, Return sends the next waiting message now, exactly as
its bubble's Send does.** A steer interrupts the agent's step and goes to the
model; a queued message goes without waiting for the turn to end.

- The next message is the earliest one waiting, the top of the waiting list:
  steers before queued messages, each in the order they were sent. A message
  on its way to the server that will wait counts, as its bubble's Send works
  on it.
- One press sends one message. A message whose Send runs is passed over, so
  the next press sends the one after it.
- It applies only when the composer shows its placeholder: it holds no text,
  not even a space, no file and no comments, and no edit of a sent message
  is open. Otherwise Return does what it does today.
- With nothing waiting, Return in an empty composer does nothing, as today.
  ⌘+Return (Ctrl+Enter on a PC) in an empty composer does nothing either.

While a message waits, the empty composer's placeholder says what Return
does:

```text
  You: Run the build                               (turn)
  Running the build...
  +------------------------------------------------+
  | Use the staging config instead                 |
  | Send  Recall  Delete              Sending soon |
  +------------------------------------------------+
  +------------------------------------------------+
  | Press ⏎ to send the next message now           |
  +------------------------------------------------+
```

The design to build from, all in `web-ui`:

- `pending-steers.ts`: `waitingSend(row): WaitingSend | undefined`, with
  `interface WaitingSend { as: 'steer' | 'queued'; id: string }`, says what
  a waiting row's Send does. The row's Send button and the composer both go
  through it.
- `useMessageRows` returns `waiting`, the waiting rows' sends in list order,
  which `AgentMessageList` exposes.
- `nextWaitingSend(waiting)` picks the first of them whose
  `sendNowOperation` is not running.
- `ChatSession` provides `provideSendNextWaiting`, as it provides the Up
  Arrow's `provideEditLastMessage`: that send, as the emit its bubble's Send
  makes, or nothing.
- `SessionComposer` calls it for Return in an empty composer and shows the
  placeholder while it offers one.

Nothing is stored, and the protocol does not change.

## Spike

None.

## Prior art

Codex CLI ignores Return in an empty composer, also while a message waits:
`handle_submission_with_time` in
[`codex-rs/tui/src/bottom_pane/chat_composer.rs`](https://github.com/openai/codex/blob/0ed34eb9ce/codex-rs/tui/src/bottom_pane/chat_composer.rs)
returns `InputResult::None` when the text and the attachments are empty, and
its waiting messages go by the turn's end. Up Arrow in an empty composer
already opens the last message's editor (0297), which is the closest
precedent: an empty composer's key acts on the message the user just sent.

## Alternatives

- Send the newest waiting message: after typing a few, the user's next
  thought is most often about the one sent first, which also goes first by
  itself. The earliest keeps the list's order.
- Send every waiting message at once: one press can't be taken back, and a
  steer interrupts a step, so each costs a press.
- Show the key only in the Send button's tooltip: it is found only by
  hovering a button the key replaces.

## Open questions

None.
