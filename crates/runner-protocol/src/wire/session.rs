//! The session a link keeps across its connections, and the frames that carry
//! it (`crates/runner-protocol/src/wire/mod.rs`).
//!
//! Between one start of a runner and one start of the backend each direction
//! of the link numbers the messages it writes, in the order it writes them,
//! and keeps each until the other end confirms it. A connection that replaces
//! another says how many messages it received, and the sender writes again
//! what follows.
//!
//! ```text
//!   byte 0      kind: 0 = the connection's own, 1 = numbered, 2 = ack
//!   numbered    | 1 | seq: u64 BE | ack: u64 BE | message |
//!   ack         | 2 | ack: u64 BE |
//!   own         | 0 | message |      hello, hello_ok, ping, pong
//! ```
//!
//! - A message either outlasts the connection or does not ([`Carriage`]). One
//!   that does not asks something of the connection that the connection's end
//!   fails, such as a Host request: a new connection must not run it late. Such
//!   a message is kept and numbered all the same, so the numbers stay whole, and
//!   a connection that replaces another writes its number again with nothing in
//!   it ([`Read::Skipped`]).
//! - Numbering happens where a message is written to the connection, after
//!   the writer chose control before input, so the numbers are the order on
//!   the wire: one cumulative acknowledgement is enough and a receiver holds
//!   nothing out of order.
//! - A message counts as received once its end handled it ([`Session::handled`]),
//!   not when it was read off the connection: what the end read and then dropped
//!   as its connection ended is not confirmed, so the other end writes it again.
//! - What is kept is what a connection accepted and the other end has not
//!   confirmed. A connection that stops taking bytes stops its writer, so that
//!   is bounded by the connection's own buffers; what waits to be written
//!   waits in the writer's queues, as it does without a session.
//! - An acknowledgement rides on every numbered message. An end that received
//!   messages and wrote none for [`ACK_DELAY`] sends a lone `ack`, as TCP
//!   delays one, so a quiet direction still lets the other release what it
//!   keeps. Acknowledging only when a reader runs dry starves under load.
//! - A pipe's WebSocket uses the same frames, for one direction of bytes.
//!
//! [`Session`] is the state of one end; it does no IO, and its owner decides
//! when a session starts anew.

use std::collections::VecDeque;
use std::time::Duration;

use bytes::Bytes;
use tokio::time::Instant;

/// How long an end that received messages and wrote none waits before it
/// sends a lone acknowledgement.
pub const ACK_DELAY: Duration = Duration::from_millis(10);
/// The bytes a numbered message's frame carries before the message.
pub const NUMBERED_HEAD_BYTES: usize = 17;

const OWN: u8 = 0;
const NUMBERED: u8 = 1;
const ACK: u8 = 2;

/// How a message travels the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carriage {
    /// It belongs to the connection, which says it again: no number covers it.
    Connection,
    /// Numbered, kept until confirmed, and written again after a drop.
    Outlasting,
    /// Numbered and kept until confirmed, but never written again: its work
    /// ends with the connection it was written to.
    Scoped,
}

/// Why a frame cannot be part of the session.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("a frame of the session is cut short")]
    Short,
    #[error("a frame of the session has the unknown kind {0}")]
    Kind(u8),
    #[error("the other end confirmed {confirmed} messages of the {sent} this end wrote")]
    Confirmed { confirmed: u64, sent: u64 },
    #[error("the other end confirmed {confirmed} messages after it confirmed {before}")]
    Regressed { confirmed: u64, before: u64 },
    #[error("message {seq} arrived where {expected} was next")]
    Gap { seq: u64, expected: u64 },
}

/// What one frame of the connection says.
#[derive(Debug, PartialEq, Eq)]
pub enum Read<'a> {
    /// A message of the connection's own, which no number covers.
    Own(&'a [u8]),
    /// The next numbered message, to hand on and then to count
    /// ([`Session::handled`]).
    Message(&'a [u8]),
    /// A numbered message already read, which a connection wrote again.
    Duplicate,
    /// The next numbered message, which was not written again: it says
    /// nothing, and is counted ([`Session::handled`]) in its turn.
    Skipped,
    /// A lone acknowledgement.
    Ack,
}

/// One end of a session: what it wrote and keeps, and what it received.
#[derive(Debug, Default)]
pub struct Session {
    /// The numbered messages the other end confirmed.
    confirmed: u64,
    /// The messages written and not confirmed, the first being number
    /// `confirmed`, with whether a new connection writes each again.
    kept: VecDeque<(Bytes, bool)>,
    /// How many of `kept` a new connection wrote again so far.
    cursor: usize,
    /// How many of the other end's numbered messages were handled: what a
    /// frame acknowledges and a new connection tells the other end.
    received: u64,
    /// The number of the other end's next message to read off the connection
    /// that serves, which runs ahead of `received` by the messages read and
    /// not handled yet.
    next: u64,
    /// Since when messages arrived that no frame written acknowledged.
    owed_since: Option<Instant>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many of the other end's messages this end received, that is,
    /// handled: what a new connection tells the other end.
    pub fn received(&self) -> u64 {
        self.received
    }

    /// One more of the other end's messages was handled, or skipped in the
    /// order it came; the next frame acknowledges it.
    pub fn handled(&mut self) {
        self.received += 1;
        self.owed_since.get_or_insert_with(Instant::now);
    }

    /// Whether a connection whose other end says it received `received`
    /// messages can carry the session on: not fewer than it confirmed, and
    /// not more than this end wrote.
    pub fn accepts(&self, received: u64) -> bool {
        self.confirmed <= received && received <= self.sent()
    }

    /// How many numbered messages this end wrote.
    pub fn sent(&self) -> u64 {
        self.confirmed + self.kept.len() as u64
    }

    /// How many messages wait for the other end's confirmation.
    pub fn unconfirmed(&self) -> usize {
        self.kept.len()
    }

    /// Starts the session anew: nothing written is kept and nothing received
    /// counts.
    pub fn restart(&mut self) {
        *self = Self::default();
    }

    /// Numbers `message`, keeps it, and answers the frame that writes it.
    /// `again` says whether a new connection writes it again.
    pub fn stamp(&mut self, message: Vec<u8>, again: bool) -> Vec<u8> {
        let seq = self.sent();
        let message = Bytes::from(message);
        let frame = self.frame(seq, &message);
        self.kept.push_back((message, again));
        frame
    }

    fn frame(&mut self, seq: u64, message: &[u8]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(NUMBERED_HEAD_BYTES + message.len());
        frame.push(NUMBERED);
        frame.extend_from_slice(&seq.to_be_bytes());
        frame.extend_from_slice(&self.received.to_be_bytes());
        frame.extend_from_slice(message);
        self.owed_since = None;
        frame
    }

    /// A connection that replaces another: the other end received
    /// `received` messages, so the ones before are confirmed and the rest are
    /// written again by [`Session::next_resend`]. It writes again what this
    /// end did not handle, so the next message to read is the first of those.
    pub fn resume(&mut self, received: u64) -> Result<(), SessionError> {
        self.release(received)?;
        self.cursor = 0;
        self.read_again();
        Ok(())
    }

    /// A connection that replaces another, for an end that only reads: the
    /// other end writes again what this end did not handle, so the next
    /// message to read is the first of those.
    pub fn read_again(&mut self) {
        self.next = self.received;
    }

    /// The next kept message to write again, framed with the confirmation as
    /// it stands now; a message that does not outlast its connection goes as
    /// its number alone.
    pub fn next_resend(&mut self) -> Option<Vec<u8>> {
        let (message, again) = self.kept.get(self.cursor)?.clone();
        let seq = self.confirmed + self.cursor as u64;
        self.cursor += 1;
        Some(self.frame(seq, if again { &message } else { &[] }))
    }

    fn release(&mut self, confirmed: u64) -> Result<(), SessionError> {
        if confirmed < self.confirmed {
            return Err(SessionError::Regressed {
                confirmed,
                before: self.confirmed,
            });
        }
        if confirmed > self.sent() {
            return Err(SessionError::Confirmed {
                confirmed,
                sent: self.sent(),
            });
        }
        while self.confirmed < confirmed {
            self.kept.pop_front();
            self.confirmed += 1;
            self.cursor = self.cursor.saturating_sub(1);
        }
        Ok(())
    }

    /// Reads one frame of the connection: releases what it confirms and says
    /// what to do with the message it carries.
    pub fn read<'a>(&mut self, frame: &'a [u8]) -> Result<Read<'a>, SessionError> {
        let (&kind, rest) = frame.split_first().ok_or(SessionError::Short)?;
        match kind {
            OWN => Ok(Read::Own(rest)),
            ACK => {
                let ack = number(rest, 0)?;
                self.release(ack)?;
                Ok(Read::Ack)
            }
            NUMBERED => {
                let seq = number(rest, 0)?;
                let ack = number(rest, 8)?;
                self.release(ack)?;
                // Whatever arrives is acknowledged, a repeat too, so the
                // other end lets go of it.
                self.owed_since.get_or_insert_with(Instant::now);
                if seq < self.next {
                    return Ok(Read::Duplicate);
                }
                if seq > self.next {
                    return Err(SessionError::Gap {
                        seq,
                        expected: self.next,
                    });
                }
                self.next += 1;
                match &rest[16..] {
                    [] => Ok(Read::Skipped),
                    message => Ok(Read::Message(message)),
                }
            }
            kind => Err(SessionError::Kind(kind)),
        }
    }

    /// When a lone acknowledgement is due, if messages wait for one.
    pub fn ack_due(&self) -> Option<Instant> {
        self.owed_since.map(|since| since + ACK_DELAY)
    }

    /// The lone acknowledgement, if one is owed.
    pub fn ack_frame(&mut self) -> Option<Vec<u8>> {
        self.owed_since.take()?;
        let mut frame = Vec::with_capacity(9);
        frame.push(ACK);
        frame.extend_from_slice(&self.received.to_be_bytes());
        Some(frame)
    }
}

fn number(bytes: &[u8], at: usize) -> Result<u64, SessionError> {
    let bytes = bytes.get(at..at + 8).ok_or(SessionError::Short)?;
    Ok(u64::from_be_bytes(bytes.try_into().expect("eight bytes")))
}

/// The frame of a message of the connection's own, which no number covers.
pub fn own_frame(message: Vec<u8>) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1 + message.len());
    frame.push(OWN);
    frame.extend_from_slice(&message);
    frame
}

/// A queue that outlives the connections that write it, for the messages of
/// work that goes on across a connection's loss: a message waits here for the
/// next connection, and a connection takes the messages in the order they
/// came.
#[derive(Clone, Default)]
pub struct Lane(std::sync::Arc<LaneInner>);

#[derive(Default)]
struct LaneInner {
    queue: std::sync::Mutex<VecDeque<super::Frame>>,
    pushed: tokio::sync::Notify,
}

impl Lane {
    pub fn push(&self, frame: super::Frame) {
        self.0
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(frame);
        self.0.pushed.notify_one();
    }

    /// The oldest message, waiting for one; cancel-safe.
    pub async fn next(&self) -> super::Frame {
        loop {
            let pushed = self.0.pushed.notified();
            tokio::pin!(pushed);
            pushed.as_mut().enable();
            if let Some(frame) = self.pop() {
                return frame;
            }
            pushed.await;
        }
    }

    /// The oldest message, if one waits.
    pub fn pop(&self) -> Option<super::Frame> {
        self.0
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
    }

    /// Drops what waits: the messages of a session that ended.
    pub fn clear(&self) {
        self.0
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}
