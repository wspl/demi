---
status: accepted
---

# A message the user sends stays in sight until it shows, or says it was not delivered

## Problem

For example, Alex opens a conversation on a phone on a slow network. The page
shows the conversation's copy on disk and reads the transcript again, which
takes forty seconds. Meanwhile Alex sends "Did you check the logs?". The
backend takes it within a second and the agent answers. On the page, the
message disappears three seconds after the send and comes back, with the
answer, once the transcript arrives. Alex sends it again, and the agent
answers twice.

The page shows a message the user sent through three owners, which nothing
keeps in step:

- The page's own row of the message on its way, which it drops when the
  backend takes the message.
- The session's waiting messages, the queue, the steers and the turn's
  starting message, which name it until the turn writes it.
- The transcript, which shows it once the page holds the block.

[A command settles on the state it changed](0023-a-command-settles-on-the-state-it-changed.md)
keeps the first two in step. The last two are followed apart, so while the
page reads the transcript again, the session says the message is written and
the transcript the page shows does not hold it yet.

Weak networks show four more faults:

- A message sent while the backend is out of reach shows Requesting for as
  long as the network is gone, with no sign that it is not sent.
- A connection that a phone's system cut without a close keeps a message for
  up to 75 seconds, the silence that judges it broken.
- After the network returns, a message waits for the page's next reconnect,
  whose wait doubles to 30 seconds.
- Send on a steer or a queued message shows nothing until the session takes
  it, a round trip later. On a phone, Send and Recall wrap onto two lines,
  28 pixels tall.

## Decision

A message the user sends is in sight from the press until the transcript the
page shows holds it, or it says that it was not delivered.

- **Delivered, not shown yet.** When the backend takes a message, the page
  keeps it, with its files, as `Conversation.delivered`, shown as the sent
  message (`PendingSubmissionState.delivered`) where no other row shows it.
  It goes once the transcript holds it, or once the page holds what the
  session says now (`runtime.caughtUp`) and nothing holds it, as a queued
  message recalled from another page. A delivered message starts nothing and
  is on its way no more.
- **Not delivered.** A message given to the session waits ten seconds for its
  confirmation, or none while the browser says it is offline. Then it is not
  delivered: a red alert stands left of the bubble, always in view, as
  Messages marks one, and `Not Delivered · Demi can’t be reached.` stands under
  it. The alert sends that message again, as a new message at the end. The
  page does not send it again by itself. A message already on its way may
  still arrive, and the session takes it once by its id. A refusal shows the
  same, with its reason and request ID; one that sending again cannot mend
  shows the alert as a mark that sends nothing, and Edit is the way on. The
  failure bar under the message and the dock's Retry are gone.

  ```text
                              (!) [ Did you check the logs? ]
                     Not Delivered · Demi can’t be reached.
  ```

- **The connection, while the user waits.** While a message waits for its
  confirmation, every two seconds the page asks each open connection for a
  pong within five seconds, as at its return, and connects each closed one at
  once (`userWaits` in `transport/liveness.ts`).
- **Send on a waiting message.** Its button shows the spinner from the press
  until the session takes the message (`sendNowOperation`). On a phone, Send
  and Recall stay on one line, 44 pixels tall.

Nothing is stored or sent differently: `delivered` lives in the page only.

## Spike

Each fault was reproduced on the running product with `bun browse`, and
measured again after the change:

| Case | Before | After |
| --- | --- | --- |
| Transcript read again over 40 s, then send | message gone for 40 s | message in sight throughout |
| Backend out of reach for 90 s | Requesting for 90 s | Not Delivered after 10 s |
| Browser offline | Requesting until online | Not Delivered at once |
| Connection silent without a close | delivered after 51 s | delivered after 7 s |
| Network back after an outage | delivered after 10–30 s | delivered after 0.25 s |
| Send on a waiting steer, 1.5 s latency | no change for 2.1 s | spinner from 16 ms |

The answer itself still shows only once the transcript read arrives, since
the page reads the earlier blocks at the same time; that is a later change.

## Prior art

- Telegram's clients keep each outgoing message as a local message by its
  `random_id`, shown with a clock until the server's copy replaces it in the
  same list, and with a red mark and Resend when it fails
  ([Telegram Desktop `HistoryItem`](https://github.com/telegramdesktop/tdesktop)).
- Signal Desktop marks a message that failed to send with a red icon and
  "Failed to send", and resends on a click
  ([Signal Desktop `Message.tsx`](https://github.com/signalapp/Signal-Desktop)).
- Both send again only when the user asks; this record does the same.

## Alternatives

- Keep the session's starting message and steers in the client until the
  transcript holds them. A page that read the agents again after a gap never
  saw the starting message, so the message vanished all the same.
- Keep sending in the background after ten seconds, with the alert as a
  status. A message the user saw marked as not sent would then arrive
  unasked.

## Open questions

None.
