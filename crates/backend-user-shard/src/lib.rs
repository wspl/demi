//! The user shard.
//!
//! The crate holds:
//!
//! - the shard threads, each user's shard with the calls into it, and the
//!   shared services every shard is given;
//! - the conversations as the agent sees them, their idle watches, and the
//!   pages' product state and synchronization channels.
//!
//! The shard implements what the Cloud, the conversations' host access, the
//! conversation permissions and the plugin host need of it (`CloudShard`,
//! `HostShard`, `PermissionShard`, `PluginShard`).
//!
//! A user's conversations, devices, workspaces and attachments belong to that
//! user alone in both instance modes. An administrator's role gives no access to
//! another user's conversations or files. Account administration and the shared
//! instance's usage report are the only records the role reaches.
//!
//! ```text
//! edge: multi-threaded runtime (axum). HTTP routes, session gate, byte paths
//!   (pipes, transfers, user streams) and the shared services: control records,
//!   conversation stores, blobs, vault, provider assembly, Cloud capacity,
//!   pending runner claims, login limiter, the registry of sync channels
//!      |  shards.of(user).call(|shard, cancel| ..)   owned data in and out
//!      |  adopt(socket)    conversation, synchronization and runner sockets
//!      |  lease and pipe ends back to the edge       transfers, user streams
//!      |  a mark in the registry        a part a page shows has changed
//! v shard threads: each a LocalRuntime hosting the shards of its pinned
//! users. One user's shard: conversations (file gates, transfers, idle
//! watches), agent trees, runner connections, pipe records, the Cloud machine,
//! titles, forks, the command router, the request rate limit, the sync
//! channels
//!      |  owned data in and out
//! v threads: one per open SQLite connection; the blocking pool for disk,
//! password hashing, and request-body and transcript serialization
//! ```
//!
//! # Where state lives
//!
//! Decisions and per-user state live in the shard; bytes and cross-user state
//! live at the edge and in the shared services.
//!
//! # No locks in admission logic
//!
//! A shard runs on one thread, so nothing else runs between two of its awaits
//! and admission logic needs no locks: a check and the state change it guards
//! happen with no await between them.
//!
//! A new transfer, for example, checks that the conversation's transfers are not
//! being closed and registers itself in the same synchronous step, so a switch
//! that closes transfers cannot miss it. Such a section carries a comment at the
//! site and a test that interleaves a competitor at that point; an await added
//! inside it reopens the race.
//!
//! # Crossing the boundary
//!
//! The boundary is crossed per request, WebSocket upgrade, stream admission or
//! change mark, never per tool call. What crosses is owned data: `Send` pipe
//! ends, leases and marks.
//!
//! - A transfer or user stream returns a lease with its pipe ends. The edge
//!   copies bytes while it holds the lease, and dropping it releases the
//!   admission in the shard, which can also end the lease itself, for example
//!   when the conversation is archived.
//! - A change a page shows crosses as a mark in the registry of open
//!   synchronization channels, from whoever committed it. The mark adds the part
//!   to each of the user's channels and wakes their tasks to read it in the
//!   shard.
//!
//! # Cancellation
//!
//! A shard call always runs to completion. When the requester leaves, the edge
//! cancels the call's token and only waits observe it, such as the wait for the
//! file gate or for a Cloud to wake, never a step after a commit: a target
//! switch that has won its storage write still restarts the conversation's idle
//! watch even if the page disconnected.
//!
//! Transitions such as switch, archive and detach run as spawned shard tasks
//! once they hold the conversation.
//!
//! # Threading and placement
//!
//! Shard code uses `Rc`, `RefCell` and local futures, and the traits it calls
//! through (`Host`, `ShellEnvironment`, `RpcHandler`, `ProviderRuntime`) have no
//! `Send` bound; shared services are `Send + Sync`.
//!
//! A user is the unit of placement at both levels, one worker process in a
//! multi-worker deployment and one shard thread inside a process. More threads
//! are therefore configuration, as long as per-user state stays in the shard and
//! cross-user state in shared services.
//!
//! # Components
//!
//! Each component owns its state and `Shard` combines them, so no component
//! holds a reference back to another. The components are the backend libraries
//! beneath, which cannot see `Shard`.
//!
//! A component whose operations need another's state, such as the Cloud, whose
//! reset holds the user's conversations, defines the narrow trait of what it
//! needs (`CloudShard`) and writes its operations as methods of `dyn
//! CloudShard`. This crate implements each such trait for `Shard`.
//!
//! Its own behavior is `impl Shard` blocks, one per module. The crate serves no
//! HTTP.

pub mod automations;
pub mod conversation;
#[cfg(feature = "testing")]
pub mod holds;
pub mod lifecycle;
pub mod provider_settings;
pub mod services;
pub mod shard;
pub mod sync;
pub mod tuning;
