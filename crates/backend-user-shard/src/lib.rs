//! The server's side of its users' cores (`crates/core/src/lib.rs`).
//!
//! The crate holds:
//!
//! - the shard threads, each running the cores of the users pinned to it,
//!   and the calls into a user's core (`pool`);
//! - the services every core of the server shares, and the server's own
//!   (`services`);
//! - what the server gives each core and keeps in it (`server_starter`);
//! - what is the server's alone: the pages' product state and
//!   synchronization channels, push notifications, conversation shares,
//!   provider entries lent to other servers, runner downloads ahead, and
//!   the server's upgrades.
//!
//! Its operations on a core are extension traits of the core, one per
//! module. The crate serves no HTTP.

pub mod cloud;
pub mod lent_media;
mod page_notifications;
pub mod pool;
pub mod prefetch;
pub mod provider_sharing;
pub mod server_starter;
pub mod services;
pub mod shares;
pub mod start_passes;
pub mod sync;
pub mod tuning;
pub mod upgrades;
