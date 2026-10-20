//! The building blocks every HTTP provider decodes its vendor's answers with.
//!
//! They are:
//!
//! - server-sent event framing;
//! - the two-step decode of payloads tagged by `type`;
//! - fields Demi only reports;
//! - the OpenAI-shaped Responses and Chat Completions streams with their
//!   mappers onto a run's events;
//! - the token counts of the Anthropic Messages stream.
//!
//! Everything a vendor sends is untrusted input, decoded into a type where it
//! enters.
//!
//! A run ends with a response only when the vendor said the reply is complete:
//! a Responses completion event, a Chat Completions `finish_reason`, or an
//! Anthropic `message_stop`. The closing `data: [DONE]` is not required, since
//! gateways leave it out. A body that ends before that is a failure of the run
//! even when a usage count came with it. A gateway that sends running totals in
//! every chunk makes a cut reply look complete, and the agent would answer the
//! next request from half a sentence.

pub mod chat_completions;
pub mod messages;
pub mod responses;
mod sse;
mod tagged;

use std::sync::Arc;

use demi_shared_types::{Clock, TokenUsage};

use crate::{FailureReader, ProviderFailure};

pub use sse::{SseError, sse_data};
pub use tagged::{
    NonEmpty, Reported, ReportedString, Tagged, TaggedWire, WireError, decode_tagged,
    decode_untagged,
};

/// What [`tagged_wire!`](crate::tagged_wire) expands to refers to, so that a
/// crate using the macro needs no dependency of its own for it.
#[doc(hidden)]
pub mod __private {
    pub use serde;
}

/// The vendor a shared stream mapper reads for.
///
/// It holds:
///
/// - its name in the failures the mapper composes, such as `Codex stream
///   error`;
/// - the reader that finds the wait in its failure records;
/// - the clock that says when a failure was received;
/// - the model the run asked for, which a response without usage names in its
///   log.
#[derive(Clone)]
pub struct Vendor {
    pub label: &'static str,
    pub reader: FailureReader,
    pub clock: Arc<dyn Clock>,
    pub model: String,
}

/// The usage an OpenAI-shaped API reports, whose input count includes what it
/// read from its cache and what it wrote to it.
///
/// Demi keeps the three apart, so both leave the input count. An absent count
/// is zero.
pub(crate) fn usage_with_cached_input(
    input: Option<u64>,
    output: Option<u64>,
    read: Option<u64>,
    written: Option<u64>,
) -> TokenUsage {
    let cache_read_tokens = read.unwrap_or(0);
    let cache_write_tokens = written.unwrap_or(0);
    TokenUsage {
        input_tokens: input
            .unwrap_or(0)
            .saturating_sub(cache_read_tokens)
            .saturating_sub(cache_write_tokens),
        output_tokens: output.unwrap_or(0),
        cache_read_tokens,
        cache_write_tokens,
    }
}

/// The input of a tool call as the vendor streamed it: the JSON value, or the
/// text itself when it is not JSON, so the agent reports the bad input
/// instead of losing the call.
pub(crate) fn tool_input(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|_| serde_json::Value::String(text.to_owned()))
}

/// The failure of a frame Demi cannot decode: no code, the source `stream`,
/// and the frame's text as its record; it is never retried automatically.
pub fn undecodable(label: &str, error: &WireError, text: &str) -> ProviderFailure {
    let message = format!("{label} API stream sent a frame Demi cannot read: {error}");
    ProviderFailure::protocol(message, text)
}
