//! What a session calls in its node (`runtime.md` § Sessions and turns): the
//! admission of an action, the harness's texts, and the tools. The node
//! assembly implements it, so a session never reaches its node otherwise, and
//! a tool never reaches into its session: it returns its outcome.

use std::sync::Arc;

use demi_core::{ToolResultContentBlock, ToolView};
use demi_gates::GateLease;
use demi_provider::ToolDefinition;
use futures_util::future::LocalBoxFuture;
use serde_json::Value;

pub(crate) trait SessionRuntime {
    /// The name every checkpoint of the session records.
    fn harness_name(&self) -> &str;

    /// Waits for the tree's admission and holds it while one action runs.
    fn enter_action(&self) -> LocalBoxFuture<'_, GateLease>;

    fn system_prompt(&self) -> LocalBoxFuture<'_, String>;

    /// The text before the content of a user turn.
    fn preamble(&self) -> LocalBoxFuture<'_, Option<String>>;

    /// The context text before a request, when the conversation's execution
    /// context changed since the node last saw it.
    fn context(&self) -> LocalBoxFuture<'_, Option<String>>;

    /// The tools the model may call.
    fn tools(&self) -> Arc<[ToolDefinition]>;

    /// Runs one call of a tool that [`tools`](Self::tools) names. Dropping
    /// the future stops the call.
    fn invoke_tool(
        &self,
        call: ToolInvocation,
    ) -> LocalBoxFuture<'_, Result<ToolOutcome, ToolFailure>>;
}

/// One call of a tool.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolInvocation {
    pub(crate) tool_use_id: String,
    pub(crate) tool_name: String,
    /// The input as the JSON value the provider supplied, or its text when
    /// that is not valid JSON; the tool validates it.
    pub(crate) input: Value,
}

/// How a tool call completed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolOutcome {
    pub(crate) output: Vec<ToolResultContentBlock>,
    pub(crate) is_error: bool,
    pub(crate) view: Option<ToolView>,
    /// The turn ends after this round of tools unless input arrived during
    /// it, as `yield` asks.
    pub(crate) stop_after_result: bool,
}

impl ToolOutcome {
    /// A call that completed as an error with this text.
    pub(crate) fn error(text: String) -> Self {
        Self {
            output: vec![ToolResultContentBlock::Text { text }],
            is_error: true,
            view: None,
            stop_after_result: false,
        }
    }
}

/// A tool that failed; its call completes as `Tool failed: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolFailure(pub(crate) String);
