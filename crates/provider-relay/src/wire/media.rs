//! A media call's route and the frames of its bodies.
//!
//! `POST {MEDIA_ROUTE}/{kind}` takes one frame, a
//! [`Call`](demi_provider_common::media::channel::Call) of the lent entry's
//! maker of the kind, and answers its one
//! [`Reply`](demi_provider_common::media::channel::Reply) frame, after an
//! empty frame every [`KEEP_ALIVE`] while the call runs. A frame is its
//! message's length, 4 bytes big-endian, then its MessagePack.

use std::time::Duration;

use bytes::{Buf as _, Bytes, BytesMut};
use serde::Serialize;

use super::{MESSAGE_BYTES, encoded};

/// The route of a borrowed entry's media, under which each kind's is.
pub const MEDIA_ROUTE: &str = "/api/relay/media";

/// The status a media call answers when the lent entry makes none of its
/// kind now, its switch off, HTTP 410 Gone.
pub const MAKES_NONE: u16 = 410;

/// How long a call's answer stays quiet at most: a proxy between the two
/// servers closes an answer that sends nothing for a minute or more.
pub const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// `message` as one frame; none as [`encoded`] says.
pub fn frame(message: &impl Serialize) -> Option<Bytes> {
    let message = encoded(message)?;
    let length = u32::try_from(message.len()).ok()?;
    let mut frame = BytesMut::with_capacity(4 + message.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&message);
    Some(frame.freeze())
}

/// The empty frame that keeps a quiet answer open.
pub fn keep_alive() -> Bytes {
    Bytes::from_static(&[0; 4])
}

/// The frames of a body as its chunks come.
#[derive(Default)]
pub struct Frames(BytesMut);

/// A frame longer than [`MESSAGE_BYTES`].
#[derive(Debug, thiserror::Error)]
#[error("a frame of {0} bytes is over the {MESSAGE_BYTES}-byte limit of one message")]
pub struct TooLong(pub usize);

impl Frames {
    pub fn push(&mut self, chunk: &[u8]) {
        self.0.extend_from_slice(chunk);
    }

    /// The next whole message, its empty frames passed over; none until one
    /// came whole.
    pub fn next(&mut self) -> Result<Option<Bytes>, TooLong> {
        loop {
            let Some(head) = self.0.get(..4) else { return Ok(None) };
            let length = u32::from_be_bytes(head.try_into().expect("a head is 4 bytes")) as usize;
            if length > MESSAGE_BYTES {
                return Err(TooLong(length));
            }
            if self.0.len() < 4 + length {
                return Ok(None);
            }
            self.0.advance(4);
            let message = self.0.split_to(length).freeze();
            if !message.is_empty() {
                return Ok(Some(message));
            }
        }
    }

    /// Whether a frame came in part, at the body's end.
    pub fn is_partial(&self) -> bool {
        !self.0.is_empty()
    }
}
