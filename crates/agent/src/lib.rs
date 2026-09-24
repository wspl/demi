//! The agent runtime (`crates-and-packages.md` § agent; behavior in
//! `runtime.md`). An [`AgentServer`] per user shard holds each open
//! conversation's [`Tree`]; a tree's [`Node`]s each run an
//! [`AgentSession`], which keeps a transcript, asks a provider runtime for the
//! next response, runs the tools the model requests and saves its checkpoint
//! through the tree store ([`AgentTreeStore`], [`SessionStore`]). A
//! [`Connection`] handles the decoded frames of one conversation socket,
//! which the backend owns.
//!
//! A product supplies the harness ([`AgentHarness`]), the provider runtimes
//! ([`ProviderResolver`]) and a tree store per conversation. Everything here
//! runs on the user's shard, a single-threaded runtime: nothing is `Send`,
//! and shared state sits in `Rc` and `RefCell` behind synchronous methods, so
//! no borrow crosses an await (`concurrency.md` § The user shard).

pub mod attachments;
mod harness;
mod ids;
mod node;
mod server;
mod session;
pub mod store;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod title;
pub mod transcript;

pub use harness::{AgentHarness, Profile, PromptContext};
pub use ids::{IdSource, RandomIds};
pub use node::Node;
pub use server::{
    AgentServer, Connection, ContentError, ContentResolver, FailureReader, FileReference, FrameRx,
    Outgoing, ProviderResolver, ResolveError, ServerConfig, ServerDeps, Tree, TreeStores,
    read_failures,
};
pub use session::{
    AgentSession, CompactionConfig, ForkError, RetryPolicy, SessionConfig, TranscriptSnapshot,
};
pub use store::{AgentTreeStore, SessionStore};
