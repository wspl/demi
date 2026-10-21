---
status: accepted
---

# The runner link is multiplexed by yamux

## Summary

The WebSocket between a backend and a runner carries one yamux session once
the runner's hello is accepted. Each command is one stream for its whole
life, each request one stream with its reply, and each end's heartbeat a
stream of its own. A stream that nobody reads fills its window and holds
only its sender; a dead network is found by the heartbeat within its
deadline, whatever the streams carry.

## Motivation

A backend and each runner share one WebSocket. Commands, their output and
input, requests and their replies, and the heartbeat travel on it as Demi's
own messages, told apart by ids, through one queue.

* The network stops passing data while the socket stays open. The backend's
  queue fills with a command's input; the heartbeat waits behind it, never
  sends, and so never times out. The device shows online and its commands
  hang, past 65 seconds in a measurement.
* A command does not read its input. Its input fills the queue, and the
  runner, unable to hold more, ends the whole connection: every command of
  the device fails.
* A stop, or a small request, waits behind all the input queued before it.

Nothing has a window of its own, and nothing is checked apart from what it
waits behind.

## Guide-level explanation

Nothing changes on screen. A device whose network stops passing data shows
offline within the heartbeat's deadline. A command that does not read its
input holds only itself: the user's other commands on the device go on, and
its Stop acts at once.

## Reference-level explanation

### The session

1. The runner connects as today and sends its hello as one WebSocket
   message, as today. The backend checks it, its release and its token,
   and answers in a WebSocket message, `UnsupportedProtocol` included.
2. Only after an accepted hello do both ends start yamux on the socket's
   binary messages: the backend as the server, the runner as the client.
   Until then the backend holds one message of at most 4 MiB for the
   connection, as before.

The session is libp2p's `yamux` 0.14.1, kept in `vendor/yamux/yamux` with
one change (below), patched in by the workspace's `[patch.crates-io]`.

### Streams

```text
stream  := lane frame*
lane    := kind:u8  id_length:u16  id        (big-endian, id ≤ 1024 bytes)
frame   := length:u32  message               (one runner wire message)
```

```rust
/// What a stream carries, named by the first bytes it sends.
enum LaneKind {
    /// Each end's own stream for what belongs to no command or request.
    Connection = 0,
    /// One command, from its start to its end.
    Command = 1,
    /// One request and its reply.
    Request = 2,
    /// Each end's heartbeat.
    Heartbeat = 3,
}

struct Lane {
    kind: LaneKind,
    /// The command's or request's id; empty for the others.
    id: String,
}
```

* **A command is one stream for its whole life.** The backend opens it with
  the command's start; the command's input follows on it. The runner sends
  the command's output, its calls and its exit back on it. Each half closes
  when its side has sent its last message; the stream ends when the command
  has ended and both halves are closed. A command never moves to another
  stream.
* **A request is one stream.** The request goes one way and its reply comes
  back on the same stream; then both halves close. A stop, a follow and a
  release of a command are requests.
* **Each end has one connection stream** for its notices, `JobCount` among
  them, which replaces `Pong`, and one heartbeat stream.
* The messages of one stream arrive in the order they were sent. Streams
  have no order between them. The sender keeps the order of a stream's
  messages also when the stream is full: what waits, waits in order.

### Windows

```rust
// vendor/yamux: the one change.
impl Config {
    /// The window a stream starts with when it is opened or accepted with
    /// no window of its own; 256 KiB, yamux's own, by default.
    fn set_initial_stream_window(&mut self, bytes: u32) -> &mut Self;
}
impl Control {
    /// Opens a stream whose receive window starts at `bytes`, announced in
    /// the frame that opens it.
    async fn open_stream_with_window(&mut self, bytes: u32) -> Result<Stream>;
}
impl Stream {
    /// Raises the stream's receive window to `bytes`, announced in one
    /// window update.
    fn raise_receive_window(&mut self, bytes: u32);
}
```

* A command's stream receives with a 16 MiB window from its first byte: the
  backend opens it with that window, and the runner raises it once it has
  read the lane. Every other stream starts at 256 KiB.
* What a window grants beyond 256 KiB counts against the link's 64 MiB, the
  first windows and yamux's growth alike; a command opened when it is spent
  starts with what is left, at least 256 KiB, and grows as yamux grows it.
* A window is a bound on bytes sent and not read, not memory set aside.

### Limits

| Limit | Value | Past it |
|---|---|---|
| Streams a side opens | 1,024 | Opening waits until one ends |
| Streams the peer opens | 1,024, and its two own | The link ends |
| Bytes of one message | 4 MiB | The link ends |
| Messages read and not yet taken, all streams | 32 MiB | Streams are read no further until taken |
| Messages held of one stream | 8 | That stream is read no further |
| A command's input, per message | 64 KiB | The input is cut into messages |

### Starting a command

* A command's manifest the runner does not hold goes first, as a request
  `Manifest { hash }`, and the command starts once the runner answers
  `ManifestInstalled`, or fails after 30 seconds. Later commands with the
  same manifest do not wait.
* A stop or a follow that reaches the runner before its command's start is
  kept for that command, for 30 seconds and for at most 256 commands.

### Liveness

Each end writes on its heartbeat stream every interval, and the peer echoes
it; an end counts the link lost when nothing at all has come from the peer
for the timeout, the deadlines of today. A Cloud machine's checkpoint pauses
the check. When a link ends, everything waiting on it returns.

## Upgrade and version skew

* The wire's version changes. A runner of an earlier version sends its
  hello as before, and the backend answers `UnsupportedProtocol` in the
  same WebSocket message as before; the runner updates itself.
* A backend of an earlier version refuses this runner's hello by its
  version, as at every release.
* Both ends are rewritten in one release; every runner updates when it
  lands.

## Performance

Measured on a test build, before and after, the median of three; through a
link with 100 ms of round trip and 8 MiB in flight:

| Measure | Before | After |
|---|---|---|
| A small request's round trip | 106–108 ms | 106 ms |
| A shell command | 113–116 ms | 114–118 ms |
| A process's raw output, 1 MiB | 0.12 s | 0.12 s |
| A process's raw output, 8 MiB | 0.16–0.17 s | 0.17 s |
| A process's raw output, 64 MiB | 0.53–0.54 s | 0.59 s |
| A command's output, 64 MiB | 0.37–0.40 s | 0.35–0.38 s |
| `cat` of 3 MiB of input and output | 113–119 ms | 226–229 ms |

On the machine, a small request's round trip goes from 0.11 to 0.25 ms, the
frames that open and close its stream.

## Drawbacks

* Input of more than 256 KiB takes a round trip more than before: a stream's
  opener learns the window the other end grants only from its answer, since
  yamux has no window for the whole connection that both ends know.
* A 64 MiB output is about a tenth slower.
* What a runner can make the backend hold unread is bounded by about
  1,026 × 256 KiB, 64 MiB of larger windows and 32 MiB of messages, about
  350 MiB, where one queue held a few megabytes and broke the link instead.
* Demi keeps a copy of yamux with a change of its own, and carries upstream
  fixes into it by hand until upstream has the change.

## Rationale and alternatives

* One queue, with a deadline on the heartbeat, priority for control messages
  and flow control for input: each a mechanism of Demi's own for what a
  multiplexer does.
* Streams sorted by kind, starts and stops on one, each command's input on
  another, its output on a third: one command spans streams with no order
  between them, and its input could come before its start.
* A command's stream given up when it goes quiet and opened again later,
  the receiver reading a lane's streams in turn: the order of those streams
  depends on which task runs first, and each end counts the streams by its
  own half, so a stream one end closed and the other still queues is
  counted by one and not the other, until the link ends or stops.
* yamux as released: a new stream starts at 256 KiB and doubles a round
  trip at a time; 1 MiB of output takes 0.33 s and 8 MiB 0.65 s at 100 ms.
* tokio-yamux grants a stream its whole window at once, but has no bound
  for the connection; h2 has both, but only its client opens streams, so it
  needs two connections.

## Spike

* The changed yamux and yamux 0.14.1 as released, linked over a pipe, each
  as the opener and as the accepter: three streams of 6 MiB each way, then
  40 streams at once, past the 64 MiB, on a debug build with yamux's own
  invariant checks. Every byte arrived and the link held.
* The first window swept at 100 ms: 256 KiB, 4, 8, 16 and 32 MiB; 16 MiB is
  the smallest at which 1 MiB and 8 MiB of output are as fast as before.

## Prior art

* Coder runs its agent's RPC over yamux over a WebSocket: each call opens a
  stream with a window of its own, and its liveness check runs apart from
  sending, with a 10-second deadline on a ping.
* frp multiplexes its control connection with yamux, with a 6 MiB stream
  window; Teleport with SSH channels; Tailscale's control connection with
  HTTP/2.
* Go's yamux, which frp uses, grants a new stream its whole window in the
  frame that opens it; SSH declares a channel's window when it opens it;
  HTTP/2 sets every stream's first window once per connection.
* Coder, frp and SSH give each command its own stream or channel for its
  whole life: opening it starts the command, its input follows on it, and
  its output and exit come back on it.

## Unresolved questions

* The change to yamux is to be offered upstream; the copy goes when
  upstream has it.
