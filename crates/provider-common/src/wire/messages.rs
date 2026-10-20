//! The token counts of the Anthropic Messages stream, which both providers
//! that read it share: the Anthropic API's own, and Claude Code, whose CLI
//! passes the API's stream events through (`crates/provider-claude-code/src/output.rs`).

use std::borrow::Cow;

use demi_shared_types::TokenUsage;
use serde::Deserialize;

use crate::{CallOutcome, VendorCall};

/// A stored tool call id as the Messages API takes a tool use's and its
/// result's.
///
/// The API allows letters, digits, `_` and `-`: each other character becomes
/// `_`, so Codex's `call_7Qm…|fc_68e…` goes as `call_7Qm…_fc_68e…`. It follows
/// from the stored id alone, so every request sends the same bytes; an id the
/// API allows goes as it is.
pub fn tool_use_id(id: &str) -> Cow<'_, str> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    if id.chars().all(allowed) {
        Cow::Borrowed(id)
    } else {
        Cow::Owned(id.chars().map(|c| if allowed(c) { c } else { '_' }).collect())
    }
}

/// An API call's token counts as one of its events states them. A count the
/// event leaves out, or states as `null`, is one it does not change.
#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
pub struct MessagesUsage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    /// The cache write by how long it is kept.
    #[serde(default)]
    pub cache_creation: Option<CacheCreation>,
}

/// The parts of a call's cache write by lifetime.
#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheCreation {
    /// The part kept for an hour (`"ttl": "1h"`).
    #[serde(default)]
    pub ephemeral_1h_input_tokens: Option<u64>,
}

impl MessagesUsage {
    fn cache_write_1h(&self) -> Option<u64> {
        self.cache_creation?.ephemeral_1h_input_tokens
    }
}

/// The token counts of the stream's latest API call.
///
/// They are its last `message_delta`'s, the API's own count of that call, each
/// count the delta leaves out as the same call's `message_start` stated it,
/// since the API's counts are cumulative over a message. A count the delta
/// states, even 0, is that count. None until a `message_delta` states counts:
/// nothing stands in for a count the API did not send.
#[derive(Debug, Default)]
pub struct CallUsage {
    /// The id of the latest call's message.
    id: Option<String>,
    /// The counts the latest call's `message_start` stated.
    started: MessagesUsage,
    /// The latest call's counts once a `message_delta` stated them.
    ended: Option<TokenUsage>,
    /// The part of the latest call's cache write kept for an hour, as its
    /// `message_delta`, else its `message_start`, stated it.
    ended_1h: u64,
}

impl CallUsage {
    /// A call's `message_start`, with its message's id and the counts it
    /// stated, if any.
    pub fn start(&mut self, id: Option<String>, counts: Option<MessagesUsage>) {
        *self = Self {
            id,
            started: counts.unwrap_or_default(),
            ..Self::default()
        };
    }

    /// A `message_delta` of the latest call, with the counts it stated.
    pub fn delta(&mut self, counts: MessagesUsage) {
        let started = self.started;
        self.ended = Some(TokenUsage {
            input_tokens: counts.input_tokens.or(started.input_tokens).unwrap_or(0),
            output_tokens: counts.output_tokens.or(started.output_tokens).unwrap_or(0),
            cache_read_tokens: counts
                .cache_read_input_tokens
                .or(started.cache_read_input_tokens)
                .unwrap_or(0),
            cache_write_tokens: counts
                .cache_creation_input_tokens
                .or(started.cache_creation_input_tokens)
                .unwrap_or(0),
        });
        self.ended_1h = counts
            .cache_write_1h()
            .or(started.cache_write_1h())
            .unwrap_or(0);
    }

    /// The latest call's counts, once a `message_delta` stated them.
    pub fn usage(&self) -> Option<TokenUsage> {
        self.ended
    }

    /// The latest call as the usage ledger keeps it, ended as `outcome`
    /// says (`crates/backend-database/src/usage.rs`).
    pub fn call(&self, outcome: CallOutcome) -> VendorCall {
        VendorCall {
            vendor_id: self.id.clone(),
            usage: self.ended,
            cache_write_1h_tokens: self.ended_1h,
            outcome,
        }
    }
}
