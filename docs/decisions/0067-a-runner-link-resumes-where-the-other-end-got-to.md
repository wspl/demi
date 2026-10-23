---
status: proposed
---

# A runner link resumes where the other end got to, and a `demi` call outlives a drop

## Problem

A script runs `demi agent spawn …` and the network blinks while the backend
answers. The runner ends the call as cancelled the moment its connection
closes (`crates/runner-jobs/src/commands/rpc.rs`), the backend stops the
call's handler with the link, and the script reads
`error: i/o error: command cancelled` with status 1. A command itself
outlives the drop, since its output is kept on the Host and read again from
where the backend got to; a call's output is made by the backend in memory,
and every message on the link written while the connection was dying, to
a call or to anything else, is lost.

## Decision

The user sees a `demi …` command in a script take longer over a drop and
end as it would have without one. It fails, with the reason on standard
error and status 1, only when the runner or the backend started again, the
call's job ended, the device was revoked, or the call's input was cut off
meanwhile.

**The link is a session that outlives its connections.** Between one start
of a runner and one start of the backend, each direction numbers the
messages it writes, in the order it writes them, and keeps each until the
other end confirms it. A connection that replaces another carries on from
the number the other end says it got to:

```text
runner                                       backend
  │ ── Hello { resume: { boot, received } } ──►│
  │ ◄── HelloOk { boot, received } ────────────│
  │ resends what follows `received`, then new  │
```

```text
each WebSocket binary message starts with its kind:
  0  the connection's own: Hello, HelloOk, HelloError, Ping, Pong
  1  numbered: seq u64 (this direction's count of messages before it),
     ack u64 (how many of the other direction's this end received),
     then the message
  2  a lone ack u64, sent when this end received messages and wrote none
     for 10 ms, as TCP delays an acknowledgement, so a quiet direction
     still lets the other release what it keeps
```

```rust
// runner-protocol

// Hello gains
pub resume: Option<Resume>,
pub struct Resume { pub session: u64, pub received: u64 }
// HelloOk gains
pub session: u64,  // a number the backend draws for each session it starts
pub received: u64, // 0 when `session` is new
// Claimed, which a newly paired runner gets, gains `session` too, so its
// first reconnect resumes.
```

- **Numbering happens where a message is written to the connection**, after
  0047 has chosen control before input, so the numbers are the order on the
  wire, one cumulative acknowledgement suffices and a receiver holds nothing
  out of order. Hello, HelloOk, Ping and Pong belong to the connection and
  carry no number.
- **What is kept** is what the connection accepted and the other end has
  not confirmed. A connection that stops taking bytes stops the writer, so
  that is bounded by the connection's own buffers and needs no number of
  its own. What waits to be written waits in 0047's queues, as today.
- **What waits while the other end is gone** is decided by kind: a call's
  messages wait for the next connection; a command's output, previews,
  file changes and log lines are dropped as today, since 0047's resync
  reads them from the Host again. Resending one already numbered is
  harmless, since the backend takes output by its offset.
- **What belongs to one connection** (a Host request, a log follow, a
  spawn) is numbered and kept like any message, but resent as an empty
  one that the receiver skips, so the numbers stay whole and nothing
  stale runs again: a request a page made fails as offline with its
  connection, and a log follow would otherwise be started twice.
- **A runner that comes back** with the same `instance`, naming the
  session the backend holds for it, takes the session over at once: the
  backend closes the old connection's reading and writing before it
  resends, with no 5-second probe of the old one. Anything else, a
  different `instance`, a session the backend no longer holds or a
  `received` above what the backend wrote, starts a new session with a new
  number, which HelloOk names, so both ends agree it is new: what the old
  one kept is dropped, its calls fail with their reason, and today's probe
  of a different runner process holds. A `received` below what the other
  end already confirmed breaks the session as well.
- **A message counts as received once its end has handled it**, not when
  it was read off the connection, so one read but dropped as a connection
  ends is resent.
- **A call outlives the connection on both ends.** The runner keeps it
  until its exit arrives; the backend keeps it and its handler on the
  device, as it keeps a device's jobs, not on the connection. A call ends
  without its exit only when its session ends, its job ends or the device
  is revoked; it has no time limit, as a job has none.
- **A call's stdout** runs over a WebSocket of its own with the same
  session, which the runner opens as the pipe's reader on a route of its
  own: unconfirmed bytes are kept and resent from where the runner got to,
  and a reader that stops reading (`demi … | head -1`, a cancelled call)
  ends it. A call's stdin stays a stream pipe; input cut off mid-call fails
  the call, since a call that has not read all of its input has done
  nothing. Other pipes, such as a file's contents or a job's output another
  Host writes for `demi host shell`, stay as they are and keep today's
  retry before the first byte.
- **A Host request in flight** (a page listing files) fails as offline when
  the connection drops, as today, so the page shows a Host as offline
  rather than waiting for it.
- The wire version is 60; a runner of an earlier version is refused and
  updates itself, as at every release, and a page sees nothing new. Each
  end logs `link.session_started`, `link.resumed` and `link.session_ended`
  with its cause, and `call.lost` with its reason.

## Spike

A session layer written for the spike, over tokio-tungstenite through a TCP
proxy that killed connections or swallowed their bytes, with 0047's two
queues on each end; numbers are loopback, for comparison only:

| Point | What ran | What it showed |
|---|---|---|
| Numbering and 0047's order | both directions full of 128 B control and 16 KiB input | numbered at writing: nothing out of order; at queueing: 95% of messages overtook a lower number, 4.7 MB held out of order |
| Control's delay | 64 KiB input both ways, a 200 B control message each ms | p99 35.3 ms without the session, 36.8 ms with it |
| Output and resync | a real runner and backend, a job printing 44,000 bytes, all connections killed 3 times | all 44,000 bytes, none missing; about 14 KB delivered twice, which the offsets absorb |
| What is kept on a silent path | the proxy stopped reading for 4 s | unconfirmed flat at 0.7–0.9 MB from 0.5 s on; the queues grew as they do today |
| What is kept at a drop | killed every 100 ms, 38 times | 23,336 messages resent, none lost or out of order; at most 3.7 MB kept |
| Acknowledging | numbers on every message, a lone `Ack` after 10 ms quiet | 12 lone acks for 389,128 messages when full; throughput −3.3% for 128 B messages, within noise for 64 KiB |
| A pipe | 64 KiB frames over a WebSocket of its own, killed every 300 ms | 72.3 MB/s against 77.4; all 4,420 frames in order and intact |
| A silently dead connection | the old connection swallowing bytes, the runner reconnecting after 400 ms | the new connection carried messages within 4 ms; 3,237 sent, 3,237 delivered, none twice; an old connection left writing starved the new one, so it is closed first |

Acknowledging only when a reader runs dry starved under load (106 MB
unconfirmed), which is why a lone `Ack` is sent on time.

## Prior art

- SignalR's stateful reconnect numbers its messages, acknowledges them with
  an `Ack` message, keeps the unacknowledged ones and starts a reconnected
  connection with a `Sequence` message saying where to resume
  ([HubProtocol](https://github.com/dotnet/aspnetcore/blob/main/src/SignalR/docs/specs/HubProtocol.md));
  it caps what it keeps with a buffer size, which Demi leaves to the
  connection's own back pressure.
- Eternal Terminal keeps a backup of what it wrote and, after a reconnect,
  resends what follows the sequence number the other end acknowledged
  ([BackedWriter](https://github.com/MisterTea/EternalTerminal/blob/master/src/base/BackedWriter.hpp)),
  with a 64 MiB cap.
- TCP delays an acknowledgement for a while when it has nothing to send
  ([RFC 1122 § 4.2.3.2](https://www.rfc-editor.org/rfc/rfc1122#section-4.2.3.2)).

## Alternatives

- **Each call resumes on its own**: numbers, a queue and deduplication in
  the call's messages. The same work again for every next thing that must
  outlive a drop.
- **A pipe resumes by its byte offset over HTTP**, a range read again with
  a backlog and an acknowledgement back over the link: a second mechanism
  for what the session already does.
- **Fail a call in flight when the connection drops** and keep the rest:
  a silent drop, the commonest one, would lose the result of a call whose
  effect already happened.

## Open questions

None.
