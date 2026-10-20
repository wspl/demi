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
//! `Backend::start` is the one composition root: it may construct, inject,
//! mount and return, and never branches on business state. Each layer of the
//! backend's crates knows only the layers beneath it:
//!
//! ```text
//! demi-backend (configuration, composition)
//!   |-- backend-providers (provider families)
//!   `-- backend-http (the HTTP edge)
//!         `-- backend-user-shard (the user's shard, conversations)
//!               |-- backend-plugins (the plugin host)
//!               |-- backend-permissions (conversation permissions)
//!               |-- backend-host-access --> backend-cloud, backend-runners
//!               |-- backend-cloud -------> backend-runners, backend-providers,
//!               |                          backend-idle-watch
//!               |-- backend-accounts
//!               `-- (as each needs) -----> backend-database, backend-blobs,
//!                                          backend-page-sync
//! ```
//!
//! A test imports each part it replaces from the library that owns it
//! (`backend-providers`, `backend-user-shard`, `backend-cloud`,
//! `backend-runners`), never through the executable. The backend returns no
//! secrets and proxies no model traffic, and it spawns neither `runsc` nor
//! image tools: every sandbox and disk operation goes to the machine manager.
//! The one credential that reaches a runner is a Claude Code account's token,
//! in the CLI's environment on the user's Cloud.

mod backend;
mod config;
pub mod families;
pub mod plugins;

pub use backend::{Backend, ShutdownError, ShutdownErrors, StartError, publish_commands};
pub use config::secret::{InstanceSecret, SecretError};
pub use config::{BackendConfig, Config, ConfigError, StorageKind};
