//! Metering: every response a conversation's runtime yields becomes a ledger
//! row before the agent sees it.
//!
//! Demi keeps two records of provider use that answer different questions:
//!
//! - The usage ledger is Demi's record of what each request consumed, by user,
//!   conversation, entry and model.
//! - A quota snapshot is the vendor's report of how much of a subscription
//!   account's plan is used.
//!
//! Neither stops a request.

pub mod meter;
