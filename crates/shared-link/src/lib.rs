//! The link: one yamux session over the runner's WebSocket, carrying the
//! runner wire's messages (`crates/runner-protocol/src/wire/mod.rs`) on
//! streams of their own. The backend and the runner both link it.
//!
//! ```text
//!   sender                                               receiver
//!   Outgoing::send ──lane──► ┌───────────────┐          ┌──────────────┐
//!     command J  ─ stream    │               │          │ reader task  │
//!     request R  ─ stream    │ yamux session │ binary   │ for each     │
//!     connection ─ stream    │ over the      ├─message──┤ stream, one  ├──► Incoming
//!     heartbeat  ─ stream    │ WebSocket     │ frames   │ budget       │    (messages)
//!                            └───────────────┘          └──────────────┘
//! ```
//!
//! # Before the session
//!
//! The runner's hello and the backend's answer are ordinary WebSocket
//! messages, read and written through a [`Handshake`]; the session starts on
//! the same socket once the answer accepts the hello ([`Handshake::start`]),
//! so a runner of an earlier version is answered as it always was. What is
//! sent on the link's [`Outgoing`] before the start waits for it.
//!
//! # Streams
//!
//! A message has a lane (`wire::Lane`, derived from the message): a command's,
//! a request's, or the connection's. Each lane has one stream for its life:
//!
//! - A command is a stream from its start to its end. The end that sends
//!   first opens it, the other answers on it, and each half closes when its
//!   end has sent its last message; the stream is gone when both halves are.
//!   A message for a command that ended is dropped, as the link remembers the
//!   last few ended commands.
//! - A request is a stream: the request one way, its reply back on the same
//!   stream. A request nobody answers is a stream of its own.
//! - Each end has a stream of its own for the connection, and one for its
//!   heartbeat, which the other end only reads.
//!
//! The order of one lane's messages is the order they were sent; there is
//! none between lanes. A sender waits for room in its lane's queue, and
//! [`Outgoing::post`], which cannot wait, queues behind what the lane holds,
//! in order.
//!
//! An end opens at most 1,024 streams at once, and opening one more waits for
//! one to end; a peer that opens more than that, besides its connection's
//! and its heartbeat, or sends a frame longer than a message may be, loses
//! the link.
//!
//! # Windows
//!
//! Each stream has a window of its own, so a stream that nobody reads fills it
//! and holds only its own sender. A stream starts at yamux's 256 KiB, and a
//! command's at 16 MiB, announced when it is opened, and by the end that
//! accepts it once it has read the lane: a command's first output crosses in a
//! round trip, and a stream reaches the rate of a gigabit through a 100 ms
//! round trip without growing. What the windows grant beyond 256 KiB counts
//! against 64 MiB for the whole link, which yamux's own growth draws on too,
//! and a command opened when it is spent starts with what is left. The link
//! uses yamux of libp2p for this, `vendor/yamux`, which adds a stream opened
//! with a window and a window raised on a stream; a peer running yamux as
//! released reads the announced windows and needs no change.
//!
//! One queue of messages of a few bytes each, and one budget of the bytes
//! read and not yet taken, serve every stream. A message holds its bytes of
//! the budget and its place among the few of its stream a user may hold
//! ([`Held`]) until the user lets go of it: a user that takes messages as
//! they come lets go at once, and one that cannot use a message yet, as the
//! runner cannot a command's input before the command reads it
//! ([`Incoming::next_held`]), keeps it, and the link reads that stream no
//! further. Its window then fills and its sender waits, and no other stream
//! does. What a stream carries is in [`framing`](crate::framing)'s
//! documentation.
//!
//! # Liveness
//!
//! Liveness is the link's own, since yamux has no deadline for its ping. Each
//! end writes a byte on its heartbeat stream every [`Liveness::interval`],
//! which the peer echoes, and counts the link lost when nothing at all has
//! arrived from the peer for [`Liveness::timeout`]: the echo, a message or a
//! window update. So a link that is busy with data but slow is not lost, a
//! link whose network stops passing data while its socket stays open is lost
//! within the timeout, and no stream's data holds the check, as it sits on a
//! stream of its own.
//!
//! An end that did not run for the whole timeout, as a machine copied by a
//! checkpoint or asleep, counts what it did not hear meanwhile as no silence:
//! its check runs after it has read what arrived, and restarts the count when
//! it fired long after it was due. [`Health::pause`] stops the check of an
//! end, and the peer's probe on demand ([`Health::answers`]) counts a paused
//! end as answering.
//!
//! # Ending
//!
//! A link ends, as a socket does, once its last user is dropped, or when a
//! user closes it ([`Outgoing::close`]). A close sends what the lanes queued,
//! has the session write it all, and only then sends the close frame, so
//! the other end reads every message before it sees the end. Everything
//! waiting on a link that ended returns.
//!
//! This crate knows nothing of a message but its lane, and nothing of the
//! socket but the bytes it carries and how it ends ([`SocketEnd`]).

mod connection;
mod framing;
mod handshake;
mod lanes;
mod outgoing;
mod socket;
#[cfg(feature = "testing")]
pub mod testing;

pub use connection::{Connection, Held, Health, Incoming, Liveness, Side};
pub use handshake::Handshake;
pub use outgoing::Outgoing;
pub use socket::{Received, Refusal, SocketEnd, Wire};
