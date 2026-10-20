//! The agent server: the agents of every conversation of one user shard.
//!
//! An [`AgentServer`] holds each open conversation's [`Tree`]. A tree's
//! [`Node`]s each run an [`AgentSession`](demi_agent_session::AgentSession)
//! with the standard tools (`demi_agent_tools`), and save their checkpoints
//! through the tree store (`demi_agent_store`). A [`Connection`] handles the
//! decoded frames of one conversation socket, which the backend owns.
//!
//! ```text
//! web app (ConversationClient)    backend: the user's shard           Host
//!   send ---------------------->  conversation socket
//!                                   -> Connection -> Tree -> root Node
//!                                        provider request;
//!                                        tool call ------------------> runner
//!   transcript_patch <----------  one outbox per connection
//!                                   tree store -> conversation database
//! ```
//!
//! A product supplies:
//!
//! - the commands, instructions, harness guide and context sources every node
//!   is assembled with;
//! - the user's subagent settings each spawn reads;
//! - the Host its shell tools reach
//!   ([`HostResolver`](demi_agent_tools::HostResolver));
//! - each conversation's model selection and the provider runtimes
//!   ([`ProviderResolver`]);
//! - a tree store per conversation.
//!
//! The agent server knows no product and no plugin.
//!
//! When a change reaches a conversation:
//!
//! - The command set may change while the server runs: a tree takes it when it
//!   opens, records its revision and keeps it until it closes, so a change
//!   reaches a conversation when its tree opens again.
//! - The profiles and the Subagents switch are asked at each spawn instead, so
//!   a change reaches the next spawn.
//! - A node's system prompt is rendered once, when the node is assembled.
//!
//! Everything here runs on the user's shard, a single-threaded runtime: nothing
//! is `Send`, and shared state sits in `Rc` and `RefCell` behind synchronous
//! methods, so no borrow crosses an await.
//!
//! The node assembly is the one place that creates a node's session; the
//! supervisor asks it for a child and never builds one. A session stores and
//! reads media only through its tree store: the product's store decides where
//! media bytes go, and `AgentServer` never sees a blob store.
//!
//! The crate:
//!
//! - depends on no concrete provider, Host implementation or user interface;
//! - owns no shell interpreter;
//! - owns no socket: the backend owns the conversation socket and hands the
//!   server decoded frames.

mod node;
mod server;
#[cfg(feature = "testing")]
pub mod testing;
pub mod small_request;
pub mod title;

pub use node::{CODE_PATHS_MAX, Node, ReplyFiles, code_path};
pub use server::{
    AgentServer, AgentsSource, AskError, Connection, ContentError, ContentResolver, FileReference, FrameRx, KeptAgents,
    Outgoing, SideCard, side_instruction,
    ProviderResolver, QuiescenceWatch, ResolveError, ResolvedFiles, RestoreError, ServerConfig,
    ServerDeps, Tree,
    TreeStores, Working,
};
