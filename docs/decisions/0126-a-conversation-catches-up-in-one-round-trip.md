---
status: proposed
---

# A conversation catches up in one round trip

Supersedes: 0120 (A page reaches the backend over one socket), the rule
that data a page sends on a lane before its `opened` is a protocol error.

## Problem

Alex opens a conversation the page keeps on disk. Its rows show in 90 ms,
then the spinner over the transcript's end turns for 0.5 to 3 seconds,
though nothing in the conversation changed. On a server 370 ms away, the
page is current only after:

1. The lane's `open` and its `opened`: one round trip.
2. The conversation's own `open` on that lane, sent only after `opened`,
   and its `opened`: a second round trip, plus the backend's opening of
   the conversation.
3. When the version the copy holds is one the backend no longer has, as
   after its restart, a `stale` answer, then a random wait of up to 2 s,
   then a read of the latest page.

The spinner shows once the page has been behind for 500 ms (0103), so the
second round trip alone makes it show for a conversation that never
changed, and the random wait makes it turn for up to 2 s more.

## Decision

The page is current one round trip after it shows a copy, plus the
backend's own opening, and a stale copy of the conversation it shows is
read at once.

* **Data follows a lane's open at once.** The page sends the
  conversation's `open` right behind the lane's `open`, without waiting
  for `opened`. The backend holds what a lane sends while it checks the
  lane's open, within the lane's credit of 8 messages, hands it to the
  lane's owner once it opens, and drops it when it refuses the open. Data
  for a lane the page never opened stays a protocol error, as do the rest
  of 0120's rules.
  * `crates/backend-http/src/connection.rs`: `Slot::Opening` holds the
    lane's pieces, put together as an open lane's are; `answered` passes
    them to the owner before anything the page sends later.
  * `packages/web-ui/src/transport/page-connection.ts`: a lane takes
    messages while it opens and sends them behind its open; it keeps them
    until `opened`, and a connection lost meanwhile sends them again
    behind the lane's open on the next.
  * `packages/web-ui/src/transport/conversation-lane.ts`: the lane's
    transport is the client's as soon as its `open` goes; a refusal ends
    it with the refusal, which the client meets as it meets one today.
* **The shown conversation reads again at once.** A `stale` answer to the
  conversation the page shows starts its read without the random wait;
  the conversations the page keeps without showing them keep the wait of
  up to 2 s, which spreads the reads of a backend restart that tells every
  page at once. The client's `staleDelayMs` option becomes `shows(node)`,
  whether the page shows the node now (null for the conversation's
  agents), and the client owns the 2 s, `STALE_SPREAD_MS`.

What an older page meets: it reloads into the new web app as the backend
starts, as after any release. A newer page never meets an older backend,
since the page comes from the backend it talks to.

## Spike

* **What each step costs.** In a development backend behind a forwarder of
  370 ms round trips, a conversation opened from its copy after the
  backend restarted showed its rows at 40 ms; the lane opened at 420 ms;
  the conversation's `opened` arrived at about 810 ms, answering `stale`;
  the read again came back 1.8 s later; the spinner turned from 637 ms to
  2,754 ms.
  On a server, from its log over three days, the conversation's `opened`
  came on average 2.8 s after a copy showed its rows, most of it a wait on
  the Host that a separate fix removes.

## Prior art

* HTTP/2 opens a stream by sending its first frame, and the client sends
  the request's data behind it without a confirmation
  ([RFC 9113 §5.1](https://www.rfc-editor.org/rfc/rfc9113#section-5.1)); a
  refused stream is reset, as a refused lane drops what it held. QUIC
  streams work the same way
  ([RFC 9000 §2.1](https://www.rfc-editor.org/rfc/rfc9000#section-2.1)).
* SSH waits for `SSH_MSG_CHANNEL_OPEN_CONFIRMATION` before a channel's
  data ([RFC 4254 §5.1](https://www.rfc-editor.org/rfc/rfc4254#section-5.1)),
  as Demi's lanes do today. Demi follows HTTP/2, since a lane's open is
  refused only for what the page can do nothing about at once.

## Alternatives

* **The conversation's `open` inside the lane's open target.** One frame
  instead of two, but the lane's check would read the conversation's
  frames, which belong to the lane's owner.
* **A longer spinner threshold.** It would hide the wait, not shorten it;
  a page that falls behind for real would say so later.

## Open questions

None.
