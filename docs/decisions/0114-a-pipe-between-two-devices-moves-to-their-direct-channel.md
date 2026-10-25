---
status: proposed
---

# A pipe between two devices moves to their direct channel

## Problem

Alex has a backend on a server abroad and two paired devices on the same
office network, a Mac and a Linux build box. An agent working on the Mac
copies a dataset to the build box:

```sh
tar c -C ~/datasets . | demi host shell --host build-box "tar x -C /data"
```

Every byte goes from the Mac to the server and back to the build box, beside
it on the same switch. The command's standard input is a pipe whose source
is the Mac's runner and whose sink is the build box's
(`crates/backend-host-access/src/host_commands.rs`): the Mac writes it to the
backend over a WebSocket, and the build box reads it from the backend. The
copy runs at the slower of the office's upload and the server's link, and
the server sends the whole dataset out again.

The two runners can already talk directly. A port forward between them opens
its connections as data channels of the pair's WebRTC peer
(`crates/runner-direct/src/pairs.rs`), but only a connection that opens while
the peer is connected:

- The first connection between a pair has the backend introduce the two
  runners and goes through the relay, since nothing waits for the peer. It
  stays on the relay for as long as it lasts, however soon the peer connects.
- So does every connection past the peer's 64 channels, and every connection
  while the pair waits to try again after a failed peer.

A pipe between two devices has no direct path at all. Two kinds exist: the
standard input and output of `demi host shell`, and the two pipes of a
forwarded connection on the relay
(`crates/backend-user-shard/src/shard/port_forwards.rs`).

## Decision

A pipe whose source and sink are two devices starts on the relay at once, as
today, moves to the pair's direct channel as soon as the pair's peer is
connected, and moves back to the relay when the channel fails. No byte is
lost or repeated across a move, and nothing waits for the peer.

The user sees nothing new. The copy above runs at the office network's speed
once the peer connects, about a second after the command starts, and a
forwarded connection that opened on the relay moves too. A pair the backend
does not let talk directly, one with a Cloud or with a device whose route is
Relay Only, keeps every pipe on the relay.

### The relay stays the pipe's spine

A moved pipe keeps both its relay legs, the source's WebSocket and the
sink's `GET` or call-output session, for as long as it lasts. They carry its
start, its end, its failure and the moves; the direct channel carries only
the bytes between two moves:

```text
source runner                   backend                    sink runner
  bytes 0..N  ---- relay ---->  pipe record  ---- relay ---->  0..N
  moved{direct, at: N} ------>  (adds the     ---------------> switches to
                                 source device)                the channel
  bytes N..M  ============= direct channel ==================>  N..M
  (channel fails; G = the last byte the sink confirmed)
  moved{relay, at: G} ------->  ----------------------------->  drops G..M it
  bytes G..   ---- relay ---->  ----------------------------->  already holds
  end ----------------------->  ----------------------------->  end
```

So everything the backend does with a pipe today stays true:

- A call still exits after its standard output drained
  (`crates/backend-remote-host/src/relay.rs`), since the end always comes on
  the relay, after the last byte.
- The backend can still fail a pipe for both ends, as a stopped `demi host
  shell` does, and both ends see the failure on their relay leg.
- A call's standard output still resumes after its connection drops
  (`crates/runner-process/src/pipes/resumed.rs`): the moves are frames of its
  session like any other.

### Who moves the pipe

The source decides every move, since only it knows what it sent and what it
must send again:

1. The backend tells the source its sink's device once both ends are
   devices that may talk directly, on the source's WebSocket. The code that
   mints the pipe says whether they may: neither is a Cloud and neither
   device's route is Relay Only.
2. The source asks the pair's path. When the peer is connected and has a free
   channel, it opens a pipe channel, writes `moved{direct, at}` on the relay
   after its last relay byte, and writes on.
3. Otherwise it writes on the relay and asks again when the pair's peer
   connects. A pipe holds its place among the pair's connections, so it keeps
   the peer up and has a failed one tried again on the pair's schedule.
4. When the channel fails, the source writes `moved{relay, at}` at the last
   byte the sink confirmed, and then sends again on the relay from there.
5. At the end on the direct channel, the source waits for the sink to confirm
   every byte, moves back to the relay at the last one, and writes the end
   there.

The sink follows: it reads the relay until `moved{direct}`, then the channel,
then the relay again from `moved{relay}`, dropping what it already holds. A
channel that arrives before its move waits for it.

The sink confirms what its consumer took with the channel's existing grants
(`crates/runner-direct/src/channel.rs`), so the source keeps at most the
channel's window of unconfirmed bytes, 256 KiB and one grant, to send again.

### Trust

The sink accepts a pipe channel only from the device the backend named in
the pipe's `moved{direct}`, which the backend writes from its own record of
the source, never from what the source wrote. A runner still accepts no peer
the backend did not introduce.

### One peer per pair, opened from either side

Today the Listen On runner of a forward offers the pair's peer and opens its
channels; the other runner only answers. A `demi host shell` has a pipe each
way, so both runners of a pair now open channels on the same peer:

- A pair keeps one peer, whichever runner offered it. Both runners open
  channels on it, as the other side of a WebRTC connection can.
- When both runners dial at once, the peer the runner whose device id sorts
  first offered stands on both sides, as WebRTC's perfect negotiation
  resolves glare. That runner refuses the other's offer while its own peer
  is out or connected, and closes a peer it answered once its own goes out;
  the other answers every offer of its and drops its own. Both sides keep
  the same peer in whatever order the offers cross.
- The pairs move out of the forwards' module into one of their own in the
  runner, which forwards and pipes share.
- A peer closes a minute after the last connection on either side ended.
  Each runner counts its own connections and the channels the peer carries,
  so neither closes a peer that the other runner's connections still use.
- A pipe outlives its runner's connection to the backend, and the pairs
  belong to that connection. A pipe that began under one connection moves
  on the pairs of the next.

### The protocol

```rust
// crates/runner-protocol/src/wire/pipe_frames.rs

pub enum PipeFrame {
    Data(Bytes),
    Idle,
    End(PipeEnd),
    /// The bytes from `at` on come over `to`; `source` is the source's
    /// device, from the backend's record.
    Moved { to: PipePath, at: u64, source: String },
}

pub enum PipePath {
    Relay,
    Direct,
}

/// What the backend tells a stream pipe's source on its WebSocket, in a
/// text message, once its sink is a device it may talk directly with.
pub struct PipeSink {
    pub sink: String,
}

/// A source's move on its WebSocket, in a text message; the backend passes
/// it on to the sink as a `Moved` frame.
pub struct PipeMove {
    pub to: PipePath,
    pub at: u64,
}

// crates/runner-protocol/src/pairs.rs, the header of a pair's channel,
// which replaces the forward's own

pub enum PairHeader {
    Forward { forward: String },
    /// A pipe's bytes from `at` on.
    Pipe { pipe: String, at: u64 },
}
```

`PipeClient` (`crates/runner-process/src/pipes.rs`) carries the moves for
every stream pipe it writes and every pipe it reads, through the runner's
pairs, which it is given as a trait object, so `demi_runner_process` keeps
knowing nothing of WebRTC. A pipe channel counts toward the peer's 64
channels; a pipe that finds none free stays on the relay and asks again when
one frees.

A forward's Listen On runner announces its connections on each path as
today, and a connection whose pipes move is counted on its new path.

A backend with runner releases serves only runners of its own release
(`crates/backend-http/src/runners.rs`), so no older runner meets a `Moved`
frame.

## Spike

Two runners on one computer reached their backend through a proxy that held
each direction to 4 MB/s and delayed it by 40 ms, as a backend abroad would.
Each copy sent 100 MB of random bytes:

| Copy                                       | Time   | Through the backend |
| ------------------------------------------ | ------ | ------------------- |
| `demi host shell`, one device Relay Only   | 25.8 s | 100 MB              |
| `demi host shell`, with the move           | 8.1 s  | 4.1 MB              |
| A forwarded connection opened on the relay | 8.3 s  | 4.7 MB              |

- Stopping the sink's runner for 60 seconds in the middle of a copy ended the
  pair's peer after 29 seconds. The source moved back to the relay, and the
  copy ended with the SHA-256 it started with.
- A first build let a runner that sorted first close a peer it had answered
  only while its own offer was out. When the two offers crossed, each runner
  kept the other's peer and closed its own, which ended the peer the other
  kept, about once a second; channels from the closed peers then reached the
  sink after its move. The rule in [One peer per pair, opened from either
  side](#one-peer-per-pair-opened-from-either-side) keeps one peer on both
  sides, and a sink takes only the channel that starts at its move's byte.

## Prior art

- **libp2p's Direct Connection Upgrade through Relay**
  ([DCUtR](https://github.com/libp2p/specs/blob/master/relay/DCUtR.md)):
  two peers start on a relayed connection, hole-punch beside it, and open
  every new stream on the direct connection. A stream already open stays on
  the relay and "will have to be recreated" once the relay closes. Demi's
  forwards do the same today; this record departs from it for pipes, whose
  first and longest transfer is the one the upgrade matters for.
- **iroh** ([QUIC multipath](https://www.iroh.computer/blog/iroh-on-QUIC-multipath))
  and **Tailscale**'s magicsock move a connection between the relay and a
  direct path beneath its streams, so an open stream moves without knowing.
  Demi has no single transport beneath its pipes to do that in: the relay is
  HTTP and WebSockets through a TLS edge, the direct path SCTP over DTLS. So
  the move happens at the pipe, with the pipe's own byte positions.
- **Multipath TCP** ([RFC 8684](https://www.rfc-editor.org/rfc/rfc8684)):
  one byte stream over several paths, with a data sequence number apart from
  each path's own and a data-level acknowledgment; data a failed path did not
  deliver is sent again on another. A pipe's `at` positions and the channel's
  grants play those parts.
- **WebRTC's perfect negotiation**
  ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/WebRTC_API/Perfect_negotiation)):
  when both sides offer at once, the "polite" one rolls its offer back. The
  device id decides which side that is.

## Alternatives

- **Keep the path a pipe opens on**, as forwards do today. The first copy
  between two devices, usually the one the user waits on, stays on the relay,
  and a channel that fails midway fails the copy.
- **Wait for the peer before a pipe starts.** A firewall or a refused peer
  would hold every `demi host shell` for the peer's ten-second timeout, which
  the direct channel's rule, that nothing waits for it, refuses.
- **Leave the relay once the pipe is direct.** The backend would lose the
  pipe's end and failure, which a call's exit and a stop rely on, and a
  failed channel would have nowhere to fall back to.
- **Carry every forwarded connection as two pipes, and drop the forward's
  own direct channel.** One way of moving bytes directly instead of two, but
  every new connection would wait a round trip to the backend before its
  first byte, 100 ms or more to a server on another continent, and a browser opens many
  connections to a forwarded development server. Forwards keep opening
  direct when the peer is up; only their relay connections become movable.
  A connection opened direct still ends when its channel fails, as today.

## Open questions

None.
