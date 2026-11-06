//! The limits a user may set on the context window Demi uses for a model:
//! any size below the model's window, of which the model menu offers a few
//! steps to pick.
//!
//! The steps and their lookups are generated into `@demicodes/protocol`, so the
//! model menu shows what a session applies.
//!
//! A limit replaces the model's window wherever Demi uses the window for
//! itself, the compaction thresholds and the usage indicator. It changes
//! nothing a vendor receives or enforces: the request limits and the check that
//! a reported usage can belong to one request still follow the model's own
//! window.

use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One limit the model menu offers, and the smallest context window it
/// offers it on. Its schema types the table the page receives in
/// `@demicodes/protocol`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextLimitStep {
    /// Tokens.
    pub tokens: u32,
    pub min_window: u32,
}

/// The steps, largest first: a window of 500,000 tokens or more offers 300K
/// and 200K, and one of 1,000,000 or more 500K as well.
pub const CONTEXT_LIMIT_STEPS: [ContextLimitStep; 3] = [
    ContextLimitStep {
        tokens: 500_000,
        min_window: 1_000_000,
    },
    ContextLimitStep {
        tokens: 300_000,
        min_window: 500_000,
    },
    ContextLimitStep {
        tokens: 200_000,
        min_window: 500_000,
    },
];

/// The limits the model menu offers below the full window of a model whose
/// context window is `window`, largest first; none for a window under
/// 500,000 tokens or an unknown one, which is zero.
pub fn context_limits(window: u32) -> impl Iterator<Item = u32> {
    CONTEXT_LIMIT_STEPS
        .into_iter()
        .filter(move |step| window >= step.min_window)
        .map(|step| step.tokens)
}

/// The limit that applies to a model whose context window is `window` when
/// the user stored `limit` on it: the limit while it is below the window,
/// and none, the full window, otherwise.
pub fn applied_context_limit(window: u32, limit: Option<u32>) -> Option<u32> {
    limit.filter(|limit| *limit < window)
}

/// The window Demi uses for a model whose context window is `window` when
/// the user stored `limit` on it.
pub fn effective_context_window(window: u32, limit: Option<u32>) -> u32 {
    applied_context_limit(window, limit).unwrap_or(window)
}

/// Whether a model whose context window is `window` offers `tokens` as a
/// subagent profile's own Context (`crates/agent-server/src/server/tree/profiles.rs`): any size
/// up to its window, the window itself being the full window.
pub fn offers_context(window: u32, tokens: u32) -> bool {
    tokens >= 1 && tokens <= window
}

/// The window Demi uses for a session on a model whose context window is
/// `window`: the profile's own Context `own` when its subagent's profile set
/// one, else the user's `limit`. Either holds while the window offers it, else
/// the full window.
///
/// A profile's choice of the full window is the window's own size, which no
/// limit equals, so it gives the full window whatever the user's limit is.
pub fn session_context_window(window: u32, own: Option<u32>, limit: Option<u32>) -> u32 {
    effective_context_window(window, own.or(limit))
}

/// A subagent profile's Context as `demi agent profiles --context` and a
/// workflow's `context:` name it, and a model's limit as `demi providers
/// context` does (`crates/agent-server/src/server/tree/profiles.rs`): `default`, the user's limit;
/// `full`, the model's whole window; or a size, such as `350K`, `1M` or
/// `272000`.
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
#[error("{0:?} is not a context: use default, full, or a size such as 350K, 1M or 272000")]
pub struct ContextWordError(String);

impl FromStr for ContextChoice {
    type Err = ContextWordError;

    fn from_str(word: &str) -> Result<Self, Self::Err> {
        match word {
            "default" => return Ok(Self::Default),
            "full" => return Ok(Self::Full),
            _ => {}
        }
        let (number, unit) = match word.strip_suffix(['K', 'k']) {
            Some(thousands) => (thousands, 1_000),
            None => match word.strip_suffix(['M', 'm']) {
                Some(millions) => (millions, 1_000_000),
                None => (word, 1),
            },
        };
        number
            .parse::<u32>()
            .ok()
            .and_then(|number| number.checked_mul(unit))
            .filter(|tokens| *tokens >= 1)
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

