//! The agent server (`crates-and-packages.md` § agent-server; behavior in
//! `runtime.md`). An [`AgentServer`] per user shard holds each open
//! conversation's [`Tree`]; a tree's [`Node`]s each run an
//! [`AgentSession`](demi_agent_session::AgentSession) with the standard tools
//! (`demi_agent_tools`), and save their checkpoints through the tree store
//! (`demi_agent_store`). A [`Connection`] handles the decoded frames of one
//! conversation socket, which the backend owns.
//!
//! A product supplies the harness
//! ([`AgentHarness`](demi_agent_tools::AgentHarness)), each conversation's
//! model selection and the provider runtimes ([`ProviderResolver`]) and a
//! tree store per conversation. Everything here
//! runs on the user's shard, a single-threaded runtime: nothing is `Send`,
//! and shared state sits in `Rc` and `RefCell` behind synchronous methods, so
//! no borrow crosses an await (`concurrency.md` § The user shard).

mod node;
mod server;
#[cfg(feature = "testing")]
pub mod testing;
pub mod title;

pub use node::Node;
pub use server::{
    AgentServer, Connection, ContentError, ContentResolver, FileReference, FrameRx, Outgoing,
    ProviderResolver, ResolveError, ResolvedFiles, RestoreError, ServerConfig, ServerDeps, Tree,
    TreeStores,
};
