//! What a stream of the link carries: the lane it belongs to, then frames.
//!
//! ```text
//! stream  := lane frame*
//! lane    := kind:u8  id_length:u16  id            (all big-endian)
//!            kind's top bit: a request that nobody answers
//! frame   := length:u32  message                   (`length` bytes of message)
//! ```
//!
//! A frame has no number: the order of a stream's frames is the order of its
//! bytes, and the link promises no order between streams.

use std::io;

use demi_runner_protocol::wire::{Lane, LaneKind};
use tokio::io::{AsyncRead, AsyncReadExt};

/// The bit of a lane's kind byte that says the request on it is not answered:
/// the end that accepts the stream ends its half at once.
const ONE_WAY: u8 = 0x80;

/// The most bytes of a lane's id.
const ID_BYTES: usize = 1024;

/// The bytes that name `lane` at the start of its stream, with whether the
/// request on it is answered.
pub(crate) fn lane_header(lane: &Lane, one_way: bool) -> Vec<u8> {
    let id = lane.id.as_bytes();
    let mut header = Vec::with_capacity(3 + id.len());
    header.push(lane.kind.code() | if one_way { ONE_WAY } else { 0 });
    header.extend_from_slice(&(id.len() as u16).to_be_bytes());
    header.extend_from_slice(id);
    header
}

/// The lane a stream names, and whether the request on it is not answered.
pub(crate) async fn read_lane<R: AsyncRead + Unpin>(stream: &mut R) -> io::Result<(Lane, bool)> {
    let mut head = [0u8; 3];
    stream.read_exact(&mut head).await?;
    let one_way = head[0] & ONE_WAY != 0;
    let kind = LaneKind::from_code(head[0] & !ONE_WAY)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "an unknown lane kind"))?;
    if one_way && kind != LaneKind::Request {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a lane that is neither a request nor answered",
        ));
    }
    let length = usize::from(u16::from_be_bytes([head[1], head[2]]));
    if length > ID_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "a lane id too long"));
    }
    let mut id = vec![0; length];
    stream.read_exact(&mut id).await?;
    let id = String::from_utf8(id)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a lane id that is no text"))?;
    Ok((Lane { kind, id }, one_way))
}

/// The bytes that start a frame of `length` bytes.
pub(crate) fn frame_header(length: usize) -> [u8; 4] {
    (length as u32).to_be_bytes()
}

/// The length of the next frame, at most `limit`; none when the stream
/// ended between frames, and an error when it ended inside one or the frame
/// is longer than the stream's lane may send.
pub(crate) async fn read_length<R: AsyncRead + Unpin>(
    stream: &mut R,
    limit: usize,
) -> io::Result<Option<usize>> {
    let mut head = [0u8; 4];
    let first = stream.read(&mut head[..1]).await?;
    if first == 0 {
        return Ok(None);
    }
    stream.read_exact(&mut head[1..]).await?;
    let length = u32::from_be_bytes(head) as usize;
    if length > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a frame of {length} bytes, over the {limit}-byte limit of its lane"),
        ));
    }
    Ok(Some(length))
}

/// The `length` bytes of a frame's message.
pub(crate) async fn read_message<R: AsyncRead + Unpin>(
    stream: &mut R,
    length: usize,
) -> io::Result<Vec<u8>> {
    let mut message = vec![0; length];
    stream.read_exact(&mut message).await?;
    Ok(message)
}
