//! The DevTools socket's bytes as tungstenite reads them, with every message
//! over the connection's limit taken out (Demi patch).
//!
//! tungstenite rejects a frame over its limit from the frame's header and
//! leaves the payload unread, so the stream is out of step and the
//! connection is lost. Here such a message is read off the socket and
//! discarded as it arrives, so it never takes more memory than a read: a
//! response becomes a small error response to the same request, and the
//! connection goes on. A message that answers no request, an event, fails
//! the read, since a lost event would leave its listeners out of date.
//!
//! Chrome sends each message as one unmasked frame. A masked frame, which a
//! server must not send, and an oversized message whose first frame is
//! within the limit pass through to tungstenite, which rejects them.

use std::fmt;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The bytes read from the socket at a time.
const READ_BYTES: usize = 64 * 1024;

/// The bytes of a discarded message kept to find the request it answers:
/// Chrome writes a response's `id` first, as `{"id":123,...`.
const PREFIX_BYTES: usize = 32;

/// The blank line that ends the HTTP response of the handshake.
const HEADERS_END: &[u8] = b"\r\n\r\n";

/// A client stream of the DevTools socket that keeps messages over `limit`
/// bytes from its reader.
pub(crate) struct LimitedFrames<S> {
    inner: S,
    buffer: Box<[u8]>,
    decoder: Decoder,
}

impl<S> LimitedFrames<S> {
    pub(crate) fn new(inner: S, limit: usize, error_code: i64) -> Self {
        Self {
            inner,
            buffer: vec![0; READ_BYTES].into_boxed_slice(),
            decoder: Decoder::new(limit as u64, error_code),
        }
    }
}

impl<S> fmt::Debug for LimitedFrames<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LimitedFrames")
            .field("limit", &self.decoder.limit)
            .finish_non_exhaustive()
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for LimitedFrames<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if buf.remaining() == 0 || this.decoder.take(buf) {
                return Poll::Ready(Ok(()));
            }
            if let Some(error) = this.decoder.failure.take() {
                return Poll::Ready(Err(error));
            }
            let mut read = ReadBuf::new(&mut this.buffer);
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut read))?;
            if read.filled().is_empty() {
                // The end of the stream.
                return Poll::Ready(Ok(()));
            }
            this.decoder.decode(read.filled());
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for LimitedFrames<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Where the decoder is in the socket's bytes.
#[derive(Debug)]
enum Position {
    /// In the handshake's HTTP response, with this many bytes of its ending
    /// blank line seen.
    Handshake(usize),
    /// In a frame's header, whose bytes so far are kept.
    Header(Vec<u8>),
    /// In a frame's payload.
    Payload(Payload),
}

#[derive(Debug)]
struct Payload {
    remaining: u64,
    /// Whether the frame belongs to a message being discarded.
    discarded: bool,
    /// Whether the frame is its message's last.
    last: bool,
}

/// A message over the limit, being discarded.
#[derive(Debug)]
struct Discarded {
    size: u64,
    prefix: Vec<u8>,
}

#[derive(Debug)]
struct Decoder {
    limit: u64,
    error_code: i64,
    position: Position,
    discarded: Option<Discarded>,
    /// Decoded bytes the reader has yet to take, from `taken` on.
    ready: Vec<u8>,
    taken: usize,
    /// The failure the reader gets once it has taken every byte before it.
    failure: Option<io::Error>,
}

impl Decoder {
    fn new(limit: u64, error_code: i64) -> Self {
        Self {
            limit,
            error_code,
            position: Position::Handshake(0),
            discarded: None,
            ready: Vec::new(),
            taken: 0,
            failure: None,
        }
    }

    /// Moves decoded bytes into `buf`, and says whether there were any.
    fn take(&mut self, buf: &mut ReadBuf<'_>) -> bool {
        let pending = &self.ready[self.taken..];
        if pending.is_empty() {
            return false;
        }
        let count = pending.len().min(buf.remaining());
        buf.put_slice(&pending[..count]);
        self.taken += count;
        if self.taken == self.ready.len() {
            self.ready.clear();
            self.taken = 0;
        }
        true
    }

    /// Decodes `input`, the next bytes of the socket, up to a failure.
    fn decode(&mut self, mut input: &[u8]) {
        while !input.is_empty() && self.failure.is_none() {
            input = match &mut self.position {
                Position::Handshake(seen) => {
                    let byte = input[0];
                    self.ready.push(byte);
                    *seen = if byte == HEADERS_END[*seen] {
                        *seen + 1
                    } else if byte == HEADERS_END[0] {
                        1
                    } else {
                        0
                    };
                    if *seen == HEADERS_END.len() {
                        self.position = Position::Header(Vec::new());
                    }
                    &input[1..]
                }
                Position::Header(header) => {
                    header.push(input[0]);
                    if let Some(frame) = FrameHeader::parse(header) {
                        let header = std::mem::take(header);
                        self.start_frame(&header, frame);
                    }
                    &input[1..]
                }
                Position::Payload(payload) => {
                    let count = input.len().min(payload.remaining.try_into().unwrap_or(usize::MAX));
                    let (bytes, rest) = input.split_at(count);
                    payload.remaining -= count as u64;
                    let discarded = payload.discarded;
                    let ended = payload.remaining == 0;
                    let last = payload.last;
                    if !discarded {
                        self.ready.extend_from_slice(bytes);
                    } else if let Some(message) = &mut self.discarded {
                        let kept = PREFIX_BYTES.saturating_sub(message.prefix.len()).min(count);
                        message.prefix.extend_from_slice(&bytes[..kept]);
                    }
                    if ended {
                        self.end_frame(discarded, last);
                    }
                    rest
                }
            };
        }
    }

    /// Begins the frame whose complete header is `header`.
    fn start_frame(&mut self, header: &[u8], frame: FrameHeader) {
        let discarded = match (&mut self.discarded, frame.opcode) {
            (Some(message), CONTINUATION) => {
                message.size = message.size.saturating_add(frame.length);
                true
            }
            (Some(_), TEXT | BINARY) => {
                self.failure = Some(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a CDP message began before the oversized one ended",
                ));
                return;
            }
            (None, TEXT | BINARY) if !frame.masked && frame.length > self.limit => {
                self.discarded = Some(Discarded {
                    size: frame.length,
                    prefix: Vec::new(),
                });
                true
            }
            _ => false,
        };
        if !discarded {
            self.ready.extend_from_slice(header);
        }
        self.position = Position::Payload(Payload {
            remaining: frame.length,
            discarded,
            last: frame.last,
        });
        if frame.length == 0 {
            self.end_frame(discarded, frame.last);
        }
    }

    fn end_frame(&mut self, discarded: bool, last: bool) {
        self.position = Position::Header(Vec::new());
        if !(discarded && last) {
            return;
        }
        let Some(message) = self.discarded.take() else {
            return;
        };
        let reason = format!(
            "the CDP message is {} bytes, more than the {} bytes a message may have",
            message.size, self.limit
        );
        match response_id(&message.prefix) {
            Some(id) => {
                let response = serde_json::json!({
                    "id": id,
                    "error": {"code": self.error_code, "message": reason},
                })
                .to_string();
                push_text_frame(&mut self.ready, response.as_bytes());
            }
            None => {
                self.failure = Some(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{reason}, and it answers no request"),
                ));
            }
        }
    }
}

const CONTINUATION: u8 = 0x0;
const TEXT: u8 = 0x1;
const BINARY: u8 = 0x2;

/// What the decoder needs of a frame header (RFC 6455 § 5.2).
#[derive(Debug)]
struct FrameHeader {
    last: bool,
    opcode: u8,
    masked: bool,
    length: u64,
}

impl FrameHeader {
    /// The header `bytes` are, once they are all of it.
    fn parse(bytes: &[u8]) -> Option<Self> {
        let [first, second, rest @ ..] = bytes else {
            return None;
        };
        let masked = second & 0x80 != 0;
        let (extended, length) = match second & 0x7f {
            126 => (2, u64::from(u16::from_be_bytes(rest.get(..2)?.try_into().ok()?))),
            127 => (8, u64::from_be_bytes(rest.get(..8)?.try_into().ok()?)),
            length => (0, u64::from(length)),
        };
        let mask = if masked { 4 } else { 0 };
        if rest.len() < extended + mask {
            return None;
        }
        Some(Self {
            last: first & 0x80 != 0,
            opcode: first & 0x0f,
            masked,
            length,
        })
    }
}

/// The `id` a response's first bytes begin with.
fn response_id(prefix: &[u8]) -> Option<u64> {
    let digits = prefix.strip_prefix(b"{\"id\":")?;
    let end = digits.iter().position(|byte| !byte.is_ascii_digit())?;
    std::str::from_utf8(&digits[..end]).ok()?.parse().ok()
}

/// Appends an unmasked, final text frame of `payload` to `bytes`.
fn push_text_frame(bytes: &mut Vec<u8>, payload: &[u8]) {
    bytes.push(0x80 | TEXT);
    match payload.len() {
        length @ 0..=125 => bytes.push(length as u8),
        length @ 126..=0xffff => {
            bytes.push(126);
            bytes.extend_from_slice(&(length as u16).to_be_bytes());
        }
        length => {
            bytes.push(127);
            bytes.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    bytes.extend_from_slice(payload);
}
