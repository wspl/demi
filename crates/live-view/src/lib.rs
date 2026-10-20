//! The Host's side of the live view where it is not about its source.
//!
//! A source of pictures and input, such as a browser tab or a simulator,
//! implements [`stream::Source`] and [`input::Deliver`]. Its viewer serves its
//! user stream with these pieces:
//!
//! ```text
//! in:   page --> frames::Reader --> the source's viewer --> input::Input
//!                                                              --> Deliver
//! out:  page <-- writer::Writer <-- delivery::Delivery <-- StreamHandle
//!                                                              <-- Source
//! ```
//!
//! # Two paths, one protocol
//!
//! The page and the sender never ask which path they are on.
//!
//! ```text
//! direct (UDP)  page == WebRTC data channels ==> runner
//!                 control: one ordered channel; video: one unordered channel
//! relay (TCP)   page --WSS--> backend --WS--> runner
//!                 everything in the one byte stream, in order
//! ```
//!
//! - Direct video chunks are reliable but unordered, so one lost packet delays
//!   only its own frame. The peer carries them without SCTP's congestion
//!   window, so the rate control in `delivery` is the only limit. A chunk may
//!   reach the page before the `stream` message announcing its generation, so
//!   the page keeps it (up to 8 MiB, frames waiting up to two seconds) rather
//!   than drop it.
//! - The relay delays everything behind a lost packet until TCP resends. That
//!   is the price of a path that needs no port but the backend's.
//!
//! # Why data channels
//!
//! A WebRTC video track was refused:
//!
//! - A track's player waits 20 to 30 ms more, and its rate control reacts
//!   slowly to a bandwidth drop.
//! - A data channel with SCTP's own window halves its rate on each loss.
//! - Encoded transforms dropped whole frames.
//!
//! At 20 Mbps, 50 ms and 0.5 % loss the unordered channel stalled 0.4 s in 20 s
//! against 11.3 s for a track.
//!
//! # No queue of pictures
//!
//! The sender never builds a queue of pictures. It keeps a window of bytes
//! unacknowledged, and a new picture replaces the one waiting before the
//! encoder. On the relay each hop forwards only what the next accepts, so a
//! slow page holds the window full rather than filling backend memory.
//!
//! The crate knows no source, no conversation and no way a source is owned.

mod bwe;
pub mod delivery;
pub mod frames;
pub mod input;
mod rate;
pub mod stream;
pub mod writer;
