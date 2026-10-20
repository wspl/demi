//! The product server, the executable `demi-backend`: it authenticates the
//! web app's requests, hosts conversation agent trees, assembles providers
//! and commands, and connects those sessions to Hosts through their runners.
//! Runners execute work on a device; the backend owns conversation state and
//! product policy. `Backend::start` with a `BackendConfig` is the entry point
//! for tests.
//!
//! Two parts appear in every path. The edge (`backend-http`) accepts
//! connections, authenticates requests and copies bytes. A user's shard
//! (`backend-user-shard`) holds everything the backend decides for one user.
//!
//! ```text
//! web app --- HTTP ------------> edge --> a shared service, or the user's shard
//!         --- sync channel ----> edge --> the user's shard
//!         --- conversation ----> edge --> the user's shard
//!             socket                        |  agent tree -> conversation db
//!                         +-----------------+------------------+
//!                         v                                    v
//!                 provider runtime               the conversation's host access
//!                   |         |                                |
//!        vendor HTTP API    process on the user's Cloud   runner connection
//!                           (Claude Code CLI) --------------> |  -> the device
//! ```
//!
//! The edge is a multi-threaded Tokio runtime. A shard thread is a
//! single-threaded one hosting the shards of every user a stable hash of the
//! user id pins to it, so all of a user's work shares one thread. Each
//! module's state lives in one place:
//!
//! - The edge holds no per-user state: parsing, authentication, ownership
//!   checks and the byte copies of file transfers, pipes and user streams.
//! - The shared services (`services`) hold what spans users or is needed
//!   before the user is known: the control service, the conversation stores,
//!   the object store, the vault, provider assembly with the model catalogs,
//!   the machine manager's client, Cloud capacity, runners waiting to be
//!   paired, login lockout and the registry of each user's synchronization
//!   channels.
//! - A user's shard holds everything else the backend decides for that user:
//!   each conversation's file gate, transfers and idle watch, the agent trees,
//!   device links and one task per runner connection, the Cloud machine, and
//!   the pages' channels. It is created by the first request for the user and
//!   loads nothing ahead of need.
//! - Each open SQLite connection has a thread of its own, and work that would
//!   stall an async thread (disk IO, password and blob hashing, serializing
//!   media, read-only conversation reads) runs on Tokio's blocking pool.
//!
//! # A deployment with several workers: not built
//!
//! The design adds a deployment where each user is pinned to one worker
//! process, a complete backend for its users, behind an off-the-shelf reverse
//! proxy that passes `Origin` and `Host` unchanged, and one internal control
//! service that alone opens `control.sqlite` (the control operations already
//! take owned, serializable input). Blobs would live in S3, each SQLite owner
//! would replicate its databases to S3, and a user's Cloud disk generation
//! would be published, atomically, where the destination can restore it.
//! Routing hints select placement; authentication still establishes identity.
//! The rule a build must keep: at most one worker serves a user at a time, so
//! before another worker takes a user, the worker that owned the user is
//! fenced, its storage and execution authority ended first, and only then
//! does the destination gain write access to the user's databases and disks.
//! A route map alone does not do that: a worker that lost a route can still
//! be saving a turn while another restores the user's databases from their
//! replicas, and the two copies diverge. How a worker is fenced, when a
//! replicated checkpoint is ready for the destination to open, and how a
//! pairing claim reaches the worker that holds the unclaimed runner are open.

mod backend;
mod config;
pub mod families;
pub mod plugins;

pub use backend::{Backend, ShutdownError, ShutdownErrors, StartError, publish_commands};
pub use config::secret::{InstanceSecret, SecretError};
pub use config::{BackendConfig, Config, ConfigError, StorageKind};
