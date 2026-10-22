---
status: proposed
---

# A runner link sends control before input, and nothing on it is cut off by a count

## Problem

A backend and each runner share one WebSocket. Bulk bytes never cross it:
a command's output is read from the Host's kept output, and a stream
between processes is an HTTP exchange of its own. What crosses it is
messages: commands' starts and input, stops, requests and their replies,
output previews and the backend's ping. On the backend they wait in one
queue of 64 frames, in the order they were sent.

- **A dead network is not seen.** The network stops passing data while the
  socket stays open. The backend's queue fills with a command's input; its
  ping waits behind it and never goes, so the deadline that would find the
  connection lost never starts. The device shows online and its commands
  hang, past 65 seconds in a measurement.
- **A stop waits.** A Stop, or a small request such as a file read, waits
  behind every input frame queued before it.
- **A command that reads no input is killed.** The runner holds 64 frames of
  a command's input; one more cancels the command, because routing a
  message never waits for work that does not read.
- **The link's queues have fixed lengths** (64 frames on the backend, 8 on
  the runner), and what each does when full is a failure or a wait that
  nothing needs.

## Decision

**Two classes of message.** Every message the backend sends to a runner is
control or input:

- *Input* is a command's or a process's stdin, and the standard error and
  exit a backend command returns to its caller on the runner (`RpcOutput`,
  `RpcExit`), which a machine may write as fast as the link takes it.
- *Control* is everything else: a start, a stop, a signal, a follow, a
  release, a request, a reply, a ping.

The backend writes control before input: a control message waits only for
the frame being written, never for queued input. A stop or a signal
therefore overtakes input queued before it, as a terminal's interrupt does:
a signal sent after a line of input may reach the command before that line.

The runner writes control (its replies and requests) before output (job and
process output, its previews, the Host's replies, file changes, exits), and
output before log lines. What the runner makes by itself never piles up: a
stream has one output preview waiting at most, and the next is made from the
newest bytes once that one is written; a process's raw output goes one frame
at a time. File-change reports do the same: a client of the Host's watches,
the backend's link or a page's direct channel, has one batch of reports
waiting at most, and the changes made meanwhile merge into the next, each
subscription once with its latest sequence.

**No queue on the link has a length limit, and nothing is cancelled or
dropped for one.** The backend's control and input queues, the runner's
inbound, control and output queues and a command's held stdin grow as far as
what is sent. The one exception is the runner's log queue of 8 frames: its
lines wait in the Host's log files, so the queue only waits for room (0060).

- a command that reads no input holds its input on the runner until it reads
  it, stops or ends; nothing else waits for it, and it is never cancelled
  for not reading;
- a sender of input never waits for the command to read.

- a request for artifacts or numbers is refused only for a duplicate id,
  a package's events are never dropped, and a backend call's events never
  end it for their number.

What crosses the link is control, typed or model-written input and output
held to one frame per stream, never bulk bytes, so nothing grows by itself;
and a connection that stops making progress is ended within a minute (below),
and what it queued with it.

**Liveness does not wait for data.** Each end counts the connection lost
when nothing at all, a message, a ping or a pong, has arrived from the other
for 60 seconds; and either end counts it lost when its socket has taken no
byte of a write for 30 seconds. Progress is measured at the socket, so a
large frame on a slow uplink is not lost while its bytes move. The backend's
ping is control, so it is never queued behind input.

**The log** tells what the link did, by 0060's rules: `link.opened`,
`link.lost` (its cause, `silence` or `write`, and how long), `link.closed`,
`link.refused`, `link.replaced`, `jobs.adopted` after a reconnect,
`output.resynced` and `output.missed`, and `message.refused`, which names a
message's kind and never its content.

**Versions.** The wire version goes up by one. An old runner and a new
backend refuse each other's hello, and every upgrade replaces the runners.

## Spike

None: the design rests on Demi's own queues and messages, nothing
uncertain.

## Prior art

- HTTP/2 and QUIC send control frames (`PING`, `RST_STREAM`) ahead of data,
  as Go's HTTP/2 write scheduler does
  ([x/net/http2](https://pkg.go.dev/golang.org/x/net/http2#WriteScheduler);
  [RFC 9113 §6.7](https://www.rfc-editor.org/rfc/rfc9113#section-6.7)).
- Demi departs from them in having no flow control at all on this link: its
  bulk bytes already travel apart, so what remains is small and slow.

## Alternatives

- **A multiplexer, yamux or qmux, under the link.** Tried: a window per
  stream and a limit for the connection cannot both hold when many streams
  go unread, and the patched libraries failed review and spike. The link
  carries messages, not bulk bytes, so it needs neither.
- **HTTP/2 over the WebSocket.** It passes, but only with two connections, one
  each way, twice the memory bound and a pump of its own, for the same two
  needs.
- **Credit for each command's input**, as SSH's channel windows. It bounds
  the memory a command that reads nothing holds, but adds a message, a state
  per command and a sender that waits, for input that is small by nature.
- **Keep cancelling a command that reads no input.** A command that is slow
  to read loses its work.

## Open questions

None.
