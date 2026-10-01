//! What a session calls in its node (`runtime.md` § Sessions and turns): the
//! admission of an action, the prompts and context, and the tools. The node
//! assembly implements it, so a session never reaches its node otherwise, and
//! a tool never reaches into its session: it returns its outcome.

use std::sync::Arc;

use demi_provider_common::{RequestLimits, ResultPart, ToolDefinition};
use demi_shared_gates::{GateLease, Reservation};
use demi_shared_types::{ModelSelection, ToolView, TurnId};
use futures_util::future::LocalBoxFuture;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub trait SessionRuntime {
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

    /// What the context sources tell the node before a request
    /// (`runtime.md` § Context), in the sources' order: `seen` is each
    /// context block the model receives, from the last compaction boundary
    /// on, oldest first, and `turn` the input turn the request belongs to.
    fn context<'a>(
        &'a self,
        seen: &'a [SeenContext<'a>],
        turn: &'a TurnId,
    ) -> LocalBoxFuture<'a, Vec<NewContext>>;

    /// The tools the model may call.
    fn tools(&self) -> Arc<[ToolDefinition]>;

    /// Runs one call of a tool that [`tools`](Self::tools) names. Dropping
    /// the future stops the call.
    fn invoke_tool(
        &self,
        call: ToolInvocation,
    ) -> LocalBoxFuture<'_, Result<ToolOutcome, ToolFailure>>;

    /// Releases what the node's tools hold, such as its shell environments
    /// and the commands they run, once its session is disposed; the session's
    /// dispose finishes only after it. Nothing by default.
    fn dispose(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// A context block the model receives, as its source sees it.
#[derive(Debug, Clone, Copy)]
pub struct SeenContext<'a> {
    pub source: &'a str,
    pub text: &'a str,
}

/// What one context source answered: a new block's source and text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewContext {
    pub source: String,
    pub text: String,
}

/// One call of a tool.
#[derive(Debug, Clone)]
pub struct ToolInvocation {
    /// The provider's id of the call, which the pages' view of a command
    /// the call starts names.
    pub tool_use_id: String,
    pub tool_name: String,
    /// The input as the JSON value the provider supplied, or its text when
    /// that is not valid JSON; the tool validates it.
    pub input: Value,
    /// The model of the request that asked for the call: the media it
    /// accepts are what a result may attach.
    pub model: ModelSelection,
    /// What that model's vendor takes in one request, which bounds the video
    /// a result may attach.
    pub request_limits: RequestLimits,
    /// The node's command-storage generation now, which a job the call
    /// starts records.
    pub generation: u64,
    /// Cancelled when the action stops: a command the call started stops
    /// with it, even after the call returned.
    pub cancel: CancellationToken,
}

/// How a tool call completed.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    /// The result, with its media's bytes, which the session stores before
    /// the result enters the transcript (`runtime.md` § Media).
    pub output: Vec<ResultPart>,
    pub is_error: bool,
    pub view: Option<ToolView>,
    /// What the session does beyond recording the result; it writes the
    /// result of an effect itself.
    pub effect: Option<ToolEffect>,
}

impl ToolOutcome {
    /// A call that completed as an error with this text.
    pub fn error(text: String) -> Self {
        Self {
            output: vec![ResultPart::Text(text)],
            is_error: true,
            view: None,
            effect: None,
        }
    }
}

/// What a tool asks of its session (`runtime.md` § Dispatch and failures):
/// a tool never reaches into its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEffect {
    /// `yield`: schedule one wakeup `duration_ms` after the action ends, and
    /// end the turn after this round of tools unless input arrived during
    /// it. The call's result says `yield scheduled` with the duration.
    ScheduleYield { duration_ms: u32 },
}

/// A tool that failed; its call completes as `Tool failed: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure(pub String);
