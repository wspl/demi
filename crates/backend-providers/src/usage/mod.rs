//! Metering and the request rate limit (`usage-and-quota.md`): every
//! response a conversation's runtime yields becomes a ledger row before the
//! agent sees it, and each user may start at most 120 provider requests in
//! any 60 seconds.

pub mod meter;
pub mod rate_limit;
