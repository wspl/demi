//! What a session calls in its node (`runtime.md` § Sessions and turns): the
//! admission of an action, the harness's texts, and the tools. The node
//! assembly implements it, so a session never reaches its node otherwise, and
//! a tool never reaches into its session: it returns its outcome.

use std::sync::Arc;

use demi_core::{ModelSelection, ToolResultContentBlock, ToolView};
use demi_gates::{GateLease, Reservation};
use demi_provider::ToolDefinition;
use futures_util::future::LocalBoxFuture;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub(crate) trait SessionRuntime {
    /// The name every checkpoint of the session records.
    fn harness_name(&self) -> &str;

    /// Waits for the tree's admission and holds it while one action runs.
    fn enter_action(&self) -> LocalBoxFuture<'_, GateLease>;

    /// Holds off every change of the node's children while an edit is
    /// prepared and committed (`message-editing.md` § Admission); refused
    /// while a child lives, starts or closes, or its completion awaits
    /// delivery. Dropping the reservation ends the hold.
    fn reserve_edit(&self) -> LocalBoxFuture<'_, Result<Option<Reservation>, String>>;

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
#[derive(Debug, Clone)]
pub(crate) struct ToolInvocation {
    pub(crate) tool_use_id: String,
    pub(crate) tool_name: String,
    /// The input as the JSON value the provider supplied, or its text when
    /// that is not valid JSON; the tool validates it.
    pub(crate) input: Value,
    /// The model of the request that asked for the call: its context window
    /// sets a shell result's preview budget, and the media it accepts what a
    /// result may attach.
    pub(crate) model: ModelSelection,
    /// The node's command-storage generation now, which a job the call
    /// starts records.
    pub(crate) generation: u64,
    /// Cancelled when the action stops: a command the call started stops
    /// with it, even after the call returned.
    pub(crate) cancel: CancellationToken,
}

/// How a tool call completed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolOutcome {
    pub(crate) output: Vec<ToolResultContentBlock>,
    pub(crate) is_error: bool,
    pub(crate) view: Option<ToolView>,
    /// What the session does beyond recording the result; it writes the
    /// result of an effect itself.
    pub(crate) effect: Option<ToolEffect>,
}

impl ToolOutcome {
    /// A call that completed as an error with this text.
    pub(crate) fn error(text: String) -> Self {
        Self {
            output: vec![ToolResultContentBlock::Text { text }],
            is_error: true,
            view: None,
            effect: None,
        }
    }
}

/// What a tool asks of its session (`runtime.md` § Dispatch and failures):
/// a tool never reaches into its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolEffect {
    /// `yield`: schedule one wakeup `duration_ms` after the action ends, and
    /// end the turn after this round of tools unless input arrived during
    /// it. The call's result says `yield scheduled` with the wakeup's id.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the yield tool of the standard tools (task 4B) makes it"
        )
    )]
    ScheduleYield { duration_ms: u32 },
}

/// A tool that failed; its call completes as `Tool failed: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolFailure(pub(crate) String);
