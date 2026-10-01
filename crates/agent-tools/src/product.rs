//! What a product supplies for its agents (`runtime.md` § Sessions and
//! turns) besides the data the agent server is given: the Host a node's shell
//! tools reach, and the context sources asked before each request.

use std::rc::Rc;

use demi_host_interface::{Host, HostError, HostErrorKind};
use demi_shared_types::{NodeId, TurnId};
use futures_util::future::LocalBoxFuture;

/// The node a question is about.
#[derive(Debug, Clone, Copy)]
pub struct NodeContext<'a> {
    /// The node whose request or tool call it is.
    pub node: &'a NodeId,
    /// The conversation's root node. Every node of a tree runs on the
    /// conversation's execution target, which belongs to the root.
    pub root: &'a NodeId,
    /// The node's working directory.
    pub cwd: &'a str,
}

/// Where a conversation's nodes run. Its futures run on the user's shard, so
/// they need not be `Send`, and the agent calls it through the concrete type.
#[expect(
    async_fn_in_trait,
    reason = "a resolver runs on the user's shard: its futures are never sent to another thread"
)]
pub trait HostResolver: 'static {
    /// The Hosts the conversations' shell tools run on. The agent only asks
    /// one for its key; the product's shell environment factory
    /// (`ServerDeps::shells`) makes a node's environment on it.
    type Host: Host;

    /// The Host a node's shell tools reach now: the conversation's current
    /// execution target (`sessions-and-targets.md` § Host operations). Two
    /// answers for the same target have equal keys, so a node keeps one
    /// shell environment per target it used. By default there is none, and a
    /// shell tool's call fails.
    async fn host(&self, _context: NodeContext<'_>) -> Result<Rc<Self::Host>, HostError> {
        Err(HostError::new(
            HostErrorKind::Unavailable,
            "this agent runs no shell tools",
        ))
    }
}

/// One source of what the model must learn before a request (`runtime.md`
/// § Context), such as the conversation's execution context or a plugin.
pub trait ContextSource {
    /// The name its blocks record: `execution`, or a plugin's id.
    fn name(&self) -> &str;

    /// What the node must learn now, or nothing. `seen` is the text of the
    /// source's own blocks the model receives, oldest first, and `turn` the
    /// input turn the request belongs to. A failure adds no block; the
    /// source is asked again before the next request.
    fn context<'a>(
        &'a self,
        node: NodeContext<'a>,
        turn: &'a TurnId,
        seen: &'a [&'a str],
    ) -> LocalBoxFuture<'a, Result<Option<String>, String>>;
}
