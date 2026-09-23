//! Server-sent events, framed by the eventsource-stream library, which
//! follows the specification, with Demi's one addition: a final event that
//! the vendor ends without its blank line is kept, because vendors end bodies
//! that way.

use bytes::Bytes;
use eventsource_stream::{EventStreamError, Eventsource};
use futures_util::{Stream, StreamExt, TryStreamExt, future, stream};

/// Why an event stream could not be read.
#[derive(Debug, thiserror::Error)]
pub enum SseError<E> {
    /// The body broke off.
    #[error("the event stream broke off: {0}")]
    Transport(E),
    /// The body is not UTF-8 text.
    #[error("the event stream is not UTF-8: {0}")]
    Utf8(std::string::FromUtf8Error),
    #[error("the event stream cannot be parsed: {0}")]
    Syntax(String),
}

impl<E> From<EventStreamError<E>> for SseError<E> {
    fn from(error: EventStreamError<E>) -> Self {
        match error {
            EventStreamError::Transport(error) => Self::Transport(error),
            EventStreamError::Utf8(error) => Self::Utf8(error),
            EventStreamError::Parser(error) => Self::Syntax(error.to_string()),
        }
    }
}

/// The data of each event of a response body: its `data:` fields joined with
/// a newline. A frame without data (a comment, `event: ping`, an empty
/// `data:`) is not an event, and a frame the body ends without a blank line
/// is still one. No mapper reads the `event:` name, so it is not carried.
pub fn sse_data<S, E>(body: S) -> impl Stream<Item = Result<String, SseError<E>>>
where
    S: Stream<Item = Result<Bytes, E>>,
{
    // The specification discards an event the body does not end; a blank
    // line after the body completes it.
    let terminator = stream::once(future::ready(Ok(Bytes::from_static(b"\n\n"))));
    body.chain(terminator)
        .eventsource()
        .map_err(SseError::from)
        // The library dispatches an empty `data:` as an event with empty
        // data; Demi's rule is that a frame without data is none.
        .try_filter_map(|event| future::ready(Ok((!event.data.is_empty()).then_some(event.data))))
}
