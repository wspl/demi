//! The least context of a conversation that compacts while idle, which the
//! user sets in Settings and the session reports from. Generated into
//! `@demicodes/protocol`, so the page's field takes what the backend does.

/// The least context, in tokens, until the user sets another.
pub const IDLE_COMPACTION_CONTEXT: u64 = 200_000;

/// The least the setting takes: below it, writing the context to the cache
/// again costs little next to the detail a summary loses.
pub const IDLE_COMPACTION_CONTEXT_FLOOR: u64 = 100_000;
