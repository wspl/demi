---
status: accepted
---

# A runner's stream pipes share one connection to the backend

## Problem

Alex forwards port 8080 of a laptop to a server at home, and the two cannot
talk directly, so the relay carries the forward. Alex's browser loads a page
of 30 small files through it. Each file is a new TCP connection, and each
connection waits about 2.5 seconds before its first byte arrives, though the
relay itself moves 2 MB/s.

Most of the wait is handshakes. A relayed connection is two stream pipes,
one each way (0114), and the runner at each end writes its pipe over a
WebSocket of its own (`crates/runner-process/src/pipes.rs`). Each such
WebSocket is a new TCP connection, TLS and an upgrade, through the user's
proxy and the TLS edge in front of the backend:

```text
listening runner         backend          connecting runner
  │ forward_open ─────────►│                     │
  │◄──── forward_opened ───│── forward_connect ──►│
  │ WebSocket handshake    │   WebSocket handshake│
  │ ~1 s                   │   ~1 s               │
  │ request bytes ────────►│────────────────────►│ (GET, open since)
  │◄───────────────────────│◄──── response bytes ─│
```

Measured from a laptop behind a proxy to a backend behind Cloudflare: TLS
alone takes 0.4 to 0.8 seconds, and a first answer on a new connection 0.8
to 1.2 seconds, against 0.4 seconds for a request on a connection already
open. The pipes the runner reads are `GET` requests on its one HTTP/2
connection, so they already cost no handshake; only the pipes it writes do.

The same handshake delays every stream pipe a runner writes: a pipe between
two devices, a job's output passed on while it runs, a device request's
answer.

## Decision

A relayed connection's first byte arrives after the round trips of opening
it and of the bytes themselves, about 1.2 seconds in the case above instead
of 2.5, and a page of 30 files loads in about half the time. Nothing changes
on screen.

### The pipe connection

Each runner holds one more WebSocket to the backend, **the pipe connection**,
`WS /api/runner/pipes`, beside its link. It opens as the link comes online,
or when a pipe needs it first, and again after it ends; a ping after 30
quiet seconds keeps it through the edge. Every stream pipe the runner writes goes
as a **lane** of it instead of a WebSocket of its own: opening one is one
frame, with no handshake.

A lane carries what the pipe's WebSocket carries today, in the same order and
with the same meaning: the pipe's bytes as binary messages, the backend's
`PipeSink` and the runner's `PipeMove` as text messages, and the pipe's end
as a close with its code and reason. Pipe frames, moves to the direct
channel and the backend's pipe store do not change.

The link stays as 0047 left it: no bulk bytes cross it. Its session keeps
every message until the other end confirms it (0067), which for a pipe's
bytes would be a second copy of the stream.

### The wire

The pipe connection speaks the page connection's frames (0120), whose codec
moves from `web-api-protocol` to a new crate, `shared-lanes`, that both
connections use:

```text
byte 0  kind
  0  connection   JSON: the connection's own, none so far
  1  open         u32 lane, JSON target                     runner -> backend
  2  opened       u32 lane                                  backend -> runner
  3  refused      u32 lane, JSON refusal                    backend -> runner
  4  data         u32 lane, u8 flags (bit 0 binary, bit 1 more follows), piece
  5  close        u32 lane, u16 code, UTF-8 reason
  6  credit       u32 lane, u16 messages                    backend -> runner
```

```rust
// runner-protocol::wire::pipe_connection
pub enum PipeLaneTarget {
    /// A stream pipe this runner writes: `PUT /api/pipes/:id`'s WebSocket.
    StreamPipe { pipe: String },
}
```

- **Data goes with the open.** A runner writes a lane's first pieces right
  after its `open`, as yamux sends data with a stream's first frame; the
  backend holds them until it answers. A `refused` drops them, and the pipe
  fails with the refusal's words as a refused WebSocket fails it today.
- **Credit paces the runner.** As on the page connection, the backend grants
  the runner one more message for each it took: a runner writes no further
  than the pipe takes. A lane starts with 16 messages of at most 64 KiB, the
  size of a stream pipe's message today, so 1 MiB, the most the relay keeps
  in flight for one pipe at 2 MB/s over a 0.5-second round trip; without a
  limit, a pipe whose sink reads slowly would hold its whole source in the
  backend's memory. The backend sends a source only its sink and its close,
  which need no credit.
- **One drop ends every lane.** A pipe connection that closes fails the
  pipes it carries, as each pipe's own WebSocket failing fails it today, and
  the runner opens it again for the pipes that follow.

### Who builds what

- `shared-lanes`: the frame codec and the putting together of a message
  from its pieces, from `crates/backend-http/src/connection.rs` and
  `crates/web-api-protocol/src/connection.rs`.
- `runner-process`: `PipeClient::stream_on` opens a lane on the runner's
  pipe connection instead of a WebSocket.
- `backend-http`: `WS /api/runner/pipes` serves lanes into the same handler
  `PUT /api/pipes/:id`'s WebSocket reaches today
  (`crates/runner-client/src/stream_pipe.rs`).
- The `PUT` WebSocket route is removed. A backend serves only runners of its
  own release (`crates/backend-http/src/runners.rs`), so no older runner
  meets its absence.

## Spike

How long a relayed connection waits, and for what, from a laptop behind a
proxy to a backend behind Cloudflare: a first answer on a new HTTPS
connection took 0.76 to 1.18 seconds, TLS 0.37 to 0.79 of it; a relayed
forward's first byte took about 2.5 seconds; the relay moved 1.5 to 2.1
MB/s once open.

## Prior art

- OpenSSH's port forwarding carries every forwarded connection as a channel
  of the one SSH connection, each with its own window
  ([RFC 4254 §5](https://www.rfc-editor.org/rfc/rfc4254#section-5)).
- frp multiplexes its proxied connections over one connection to the server
  with [yamux](https://github.com/hashicorp/yamux/blob/master/spec.md),
  whose streams open with data in their first frame and grant windows both
  ways.
- cloudflared carries every proxied request over a few long-lived HTTP/2 or
  QUIC connections to the edge.
- chisel runs SSH over one WebSocket, which passes the same TLS edges ours
  does.

Like all four, a runner opens one long-lived connection and multiplexes. It
departs from yamux and SSH in counting credit in messages and granting it
one way, as the page connection does, so one codec serves both.

## Alternatives

- **The link carries the pipes.** No second connection, but bulk bytes would
  wait behind and hold up control (0047), and the link's session would keep a
  copy of every byte until confirmed (0067).
- **WebSockets over HTTP/2** ([RFC 8441](https://www.rfc-editor.org/rfc/rfc8441)),
  on the runner's HTTP/2 connection. No protocol of our own, but it needs the
  edge in front of the backend to offer it, and the reverse proxies
  self-hosters use, such as nginx, do not.
- **Streaming `PUT` bodies on HTTP/2.** A TLS edge such as Cloudflare's holds
  a request's body until it ends or passes about 2 MB, which is why stream
  pipes are WebSockets (`crates/runner-protocol/src/wire/pipe_frames.rs`).
- **Fold the forward's `forward_open` round trip into the lane's open.** It
  would save one more round trip for forwards only, and change how the
  backend makes a forward's pipes. Left for later.

## Open questions

None.
