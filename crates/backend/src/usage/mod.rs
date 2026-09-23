//! Metering and the request rate limit (`usage-and-quota.md`): every
//! response a conversation's runtime yields becomes a ledger row before the
//! agent sees it, and each user may start at most 120 provider requests in
//! any 60 seconds.

#[cfg_attr(not(test), expect(dead_code, reason = "a conversation's runtime is metered once the agent runs it (4D)"))]
pub(crate) mod meter;
#[cfg_attr(not(test), expect(dead_code, reason = "a conversation's runtime counts against it once the agent runs it (4D)"))]
pub(crate) mod rate_limit;
