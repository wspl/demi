//! Metering and the request rate limit: every
//! response a conversation's runtime yields becomes a ledger row before the
//! agent sees it, and each user may start at most 120 provider requests in
//! any 60 seconds.
//!
//! Demi keeps two records of provider use that answer different questions: the
//! usage ledger is Demi's record of what each request consumed, by user,
//! conversation, entry and model, and a quota snapshot is the vendor's report
//! of how much of a subscription account's plan is used. Neither stops a
//! request; the rate limit is the one limit Demi enforces itself.

pub mod meter;
pub mod rate_limit;
