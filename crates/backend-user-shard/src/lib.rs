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
//! ```text
//! edge: multi-threaded runtime (axum). HTTP routes, session gate, byte paths
//!   (pipes, transfers, user streams) and the shared services: control records,
//!   core stores, blobs, vault, provider registry, Cloud capacity, pending
//!   runner claims, login limiter, the registry of sync channels
//!      |  shards.of(user).call(|core, cancel| ..)   owned data in and out
//!      |  adopt(socket)    conversation, synchronization and runner sockets
//!      |  lease and pipe ends back to the edge       transfers, user streams
//!      |  a mark in the registry        a part a page shows has changed
//! v shard threads: each a LocalRuntime running the cores of its pinned
//! users, each with the server's side kept in it (`ServerSide`, `CloudSide`)
//!      |  owned data in and out
//! v threads: one per open SQLite connection; the blocking pool for disk,
//! password hashing, and request-body and transcript serialization
//! ```
//!
//! Decisions and per-user state live in the core; bytes and cross-user state
//! live at the edge and in the shared services. A user is the unit of
//! placement at both levels, one worker process in a multi-worker deployment
//! and one shard thread inside a process. More threads are therefore
//! configuration, as long as per-user state stays in the core and cross-user
//! state in shared services.
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
