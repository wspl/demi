//! What a product supplies for its agents (`runtime.md` § Sessions and
//! turns): the harness names a node's system prompt, the text added to each
//! of its user turns, the context text that announces a change of the
//! conversation's execution context, and the commands its shell offers.
//! Demi has one harness, the coding agent.

use std::rc::Rc;

use demi_core::NodeId;
use demi_shell::CommandSet;

/// The node a harness hook is asked about.
#[derive(Debug, Clone, Copy)]
pub struct PromptContext<'a> {
    /// The node whose request or turn it is.
    pub node: &'a NodeId,
    /// The conversation's root node. Every node of a tree runs on the
    /// conversation's execution target, which belongs to the root.
    pub root: &'a NodeId,
    /// The node's working directory.
    pub cwd: &'a str,
}

/// A product's agent. Its hooks run on the user's shard, so their futures
/// need not be `Send`, and the agent calls them through the concrete type.
#[expect(
    async_fn_in_trait,
    reason = "a harness runs on the user's shard: its futures are never sent to another thread"
)]
pub trait AgentHarness: 'static {
    /// The harness's name, which every checkpoint records: a node another
    /// harness saved is not restored.
    fn name(&self) -> &str;

    /// The commands a node's shell offers. A node renders their help into
    /// its system prompt once, when it is assembled.
    fn commands(&self) -> Rc<CommandSet>;

    /// The system prompt of a node's requests. `commands` is the rendered
    /// help of the node's commands, empty when it has none.
    async fn system_prompt(&self, context: PromptContext<'_>, commands: &str) -> String;

    /// The text the model receives before the content of each user turn;
    /// none by default.
    async fn preamble(&self, _context: PromptContext<'_>) -> Option<String> {
        None
    }

    /// Before each request: the text that tells the model the
    /// conversation's execution context changed since the node last saw it,
    /// such as a target switch; none when nothing changed, and by default.
    async fn context(&self, _context: PromptContext<'_>) -> Option<String> {
        None
    }
}
