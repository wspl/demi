---
status: proposed
---

# A command call outlives its program's reconnection

## Problem

An agent runs `chatbot ask-approval`, which posts a card to the group and
waits minutes for a person to tap it. The bot's program restarts for a
release meanwhile. Its socket closes, and the call fails at once: the agent
reads that the integration's program went away, though the program is back
half a minute later and the person taps the card after. A new call waits
for a program as long as a dropped runner counts as online, 10 seconds
(`tuning.rs`, `returning`), so the model soon learns that the integration is
unreachable; a call already sent does not wait at all.

## Decision

A call in flight waits for its program until the agent stops it, where
0152 failed the calls of a connection that closes.

* When the socket a call was sent on closes, the call stays, keeping its
  request. Once a socket of the integration declares again, after its
  `declared` event, or at once when another socket serves the plugin,
  Demi sends it the call's `request` again, under the same `id`, with
  `resent: true`; its later `port` messages and its `reply` come from that
  socket, and a stop's `cancel` goes to it.
* It has no deadline of its own, as a call has none while its program is
  connected: its shell call goes to the background as any long command
  does, and a stop of the command, the agent or the conversation ends it
  as it ends any command.
  A new call keeps its 10 seconds, which tell the model at once that no
  program serves the integration.
* What already happened stays: output the call wrote reaches the agent
  once, and input it read is not read again; a resent call's next
  `read_stdin` reads on from there. Input read for a `read_stdin` whose
  socket closed before its answer answers the next socket's first one. A
  call the agent stopped meanwhile is not resent.
* The program makes its work idempotent by the `id`: a resent request is
  the same call, which it may find done, as a card it posted before its
  restart. No other request takes the id, also after the backend restarts:
  ids count on from the clock's milliseconds times 1000, below 2^53.

## Spike

None: a call waits on its reply channel, which a closed socket no longer
fails (`integration_plugins.rs`, `IntegrationEvents::closed`).

## Prior art

* A runner's commands run on through its update and reconnection (0174).
* AWS SQS gives a message back to another consumer once its visibility
  timeout passes without an acknowledgment; the consumer makes its work
  idempotent by the message's id.

## Alternatives

* **The program resumes the call itself**, naming its id after it
  reconnects: a program that restarted has lost what it knew of the call.
* **A declared timeout per command**, which the asker proposed for long
  calls: a call already has no limit but the agent's stop.

## Open questions

None.
