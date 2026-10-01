//! The building blocks every HTTP provider decodes its vendor's answers with
//! (`providers.md` § Reading vendor input): server-sent event framing, the
//! two-step decode of payloads tagged by `type`, fields Demi only reports,
//! and the OpenAI-shaped Responses and Chat Completions streams with their
//! mappers onto a run's events.

pub mod chat_completions;
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

/// The vendor a shared stream mapper reads for: its name in the failures the
/// mapper composes, such as `Codex stream error`, the reader that finds the
/// wait in its failure records, and the clock that says when a failure was
/// received.
#[derive(Clone)]
pub struct Vendor {
    pub label: &'static str,
    pub reader: FailureReader,
    pub clock: Arc<dyn Clock>,
}

/// The usage an OpenAI-shaped API reports, whose input count includes what
/// it read from its cache and what it wrote to it: Demi keeps the three
/// apart, so both leave the input count (`providers.md` § Usage). An absent
/// count is zero.
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
/// instead of losing the call (`runtime.md` § Replay).
pub(crate) fn tool_input(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|_| serde_json::Value::String(text.to_owned()))
}

/// The failure of a frame Demi cannot decode: no code, the source `stream`,
/// and the frame's text as its record; it is never retried automatically.
pub fn undecodable(label: &str, error: &WireError, text: &str) -> ProviderFailure {
    let message = format!("{label} API stream sent a frame Demi cannot read: {error}");
    ProviderFailure::protocol(message, text)
}
