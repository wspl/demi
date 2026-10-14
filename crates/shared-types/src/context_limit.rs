//! The limits a user may set on the context window Demi uses for a model
//! (`models.md` § Context limit). The steps and their lookups are generated
//! into `@demicodes/protocol`, so the model menu offers what a session
//! applies.

use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One limit a user may set, and the smallest context window that offers
/// it. Its schema types the table the page receives in
/// `@demicodes/protocol`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextLimitStep {
    /// Tokens.
    pub tokens: u32,
    pub min_window: u32,
}

/// The steps, largest first: a window over 500,000 tokens offers 300K and
/// 200K, and one of 1,000,000 or more 500K as well.
pub const CONTEXT_LIMIT_STEPS: [ContextLimitStep; 3] = [
    ContextLimitStep {
        tokens: 500_000,
        min_window: 1_000_000,
    },
    ContextLimitStep {
        tokens: 300_000,
        min_window: 500_001,
    },
    ContextLimitStep {
        tokens: 200_000,
        min_window: 500_001,
    },
];

/// The limits a model whose context window is `window` offers below its
/// full window, largest first; none for a window of at most 500,000 tokens
/// or an unknown one, which is zero.
pub fn context_limits(window: u32) -> impl Iterator<Item = u32> {
    CONTEXT_LIMIT_STEPS
        .into_iter()
        .filter(move |step| window >= step.min_window)
        .map(|step| step.tokens)
}

/// The limit that applies to a model whose context window is `window` when
/// the user stored `limit` on it: the limit while the window offers it, and
/// none, the full window, otherwise.
pub fn applied_context_limit(window: u32, limit: Option<u32>) -> Option<u32> {
    limit.filter(|limit| context_limits(window).any(|offered| offered == *limit))
}

/// The window Demi uses for a model whose context window is `window` when
/// the user stored `limit` on it.
pub fn effective_context_window(window: u32, limit: Option<u32>) -> u32 {
    applied_context_limit(window, limit).unwrap_or(window)
}

/// Whether a model whose context window is `window` offers `tokens` as a
/// subagent profile's own Context (`subagents.md` § Profiles): its full
/// window or one of its limits, for a window that has a Context row at all.
pub fn offers_context(window: u32, tokens: u32) -> bool {
    let mut limits = context_limits(window).peekable();
    limits.peek().is_some() && (tokens == window || limits.any(|limit| limit == tokens))
}

/// The window Demi uses for a session on a model whose context window is
/// `window`: the profile's own Context `own` when its subagent's profile
/// set one, else the user's `limit`; either while the window offers it,
/// else the full window. A profile's choice of the full window is the
/// window's own size, which no limit equals, so it gives the full window
/// whatever the user's limit is.
pub fn session_context_window(window: u32, own: Option<u32>, limit: Option<u32>) -> u32 {
    effective_context_window(window, own.or(limit))
}

/// Whether `tokens` is a limit some window offers, which is what a user may
/// store.
pub fn is_context_limit(tokens: u32) -> bool {
    CONTEXT_LIMIT_STEPS.iter().any(|step| step.tokens == tokens)
}

/// A subagent profile's Context as `demi agent profiles --context` and a
/// workflow's `context:` name it (`subagents.md` § Profiles): `default`,
/// the user's limit; `full`, the model's whole window; or a limit, such as
/// `500K`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum ContextChoice {
    Default,
    Full,
    Limit(u32),
}

impl ContextChoice {
    /// What a profile on a model whose window is `window` stores: none for
    /// the user's limit, else the tokens of its own Context.
    pub fn tokens(self, window: u32) -> Option<u32> {
        match self {
            Self::Default => None,
            Self::Full => Some(window),
            Self::Limit(tokens) => Some(tokens),
        }
    }
}

/// Why a word names no Context.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a context: use full, {limits} or default", limits = limit_words())]
pub struct ContextWordError(String);

impl FromStr for ContextChoice {
    type Err = ContextWordError;

    fn from_str(word: &str) -> Result<Self, Self::Err> {
        match word {
            "default" => return Ok(Self::Default),
            "full" => return Ok(Self::Full),
            _ => {}
        }
        word.strip_suffix(['K', 'k'])
            .and_then(|thousands| thousands.parse::<u32>().ok())
            .and_then(|thousands| thousands.checked_mul(1000))
            .filter(|tokens| is_context_limit(*tokens))
            .map(Self::Limit)
            .ok_or_else(|| ContextWordError(word.to_owned()))
    }
}

impl TryFrom<String> for ContextChoice {
    type Error = ContextWordError;

    fn try_from(word: String) -> Result<Self, Self::Error> {
        word.parse()
    }
}

/// `tokens` as the Context row says it: `500K`, `1M`, or the number of
/// tokens for a window that is neither.
pub fn context_word(tokens: u32) -> String {
    if tokens % 1_000_000 == 0 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens % 1000 == 0 {
        format!("{}K", tokens / 1000)
    } else {
        format!("{tokens} tokens")
    }
}

/// The limits a profile may name, largest first: `500K, 300K, 200K`.
fn limit_words() -> String {
    let words: Vec<String> = CONTEXT_LIMIT_STEPS.iter().map(|step| context_word(step.tokens)).collect();
    words.join(", ")
}

/// What a model whose window is `window` offers a profile, as a refusal
/// lists it: `full (1M), 500K, 300K, 200K`; none for a window without a
/// Context row.
pub fn offered_contexts(window: u32) -> Option<String> {
    let limits: Vec<String> = context_limits(window).map(context_word).collect();
    (!limits.is_empty()).then(|| format!("full ({}), {}", context_word(window), limits.join(", ")))
}
