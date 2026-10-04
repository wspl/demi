//! The limits a user may set on the context window Demi uses for a model
//! (`models.md` § Context limit). The steps and their lookups are generated
//! into `@demicodes/protocol`, so the model menu offers what a session
//! applies.

use schemars::JsonSchema;
use serde::Serialize;

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

/// Whether `tokens` is a limit some window offers, which is what a user may
/// store.
pub fn is_context_limit(tokens: u32) -> bool {
    CONTEXT_LIMIT_STEPS.iter().any(|step| step.tokens == tokens)
}
