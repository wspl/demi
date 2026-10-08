//! What a session calls in its node (`runtime.md` § Sessions and turns): the
//! admission of an action, the prompts and context, and the tools. The node
//! assembly implements it, so a session never reaches its node otherwise, and
//! a tool never reaches into its session: it returns its outcome.

use std::sync::Arc;

use demi_provider_common::{RequestLimits, ResultPart, ToolDefinition};
use demi_shared_gates::{GateLease, Reservation};
use demi_shared_types::{CommandId, ModelSelection, ToolView, TurnId, WakeupCommand};
use futures_util::future::LocalBoxFuture;
use serde_json::Value;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub trait SessionRuntime {
    /// Waits for the tree's admission and holds it while one action runs.
    fn enter_action(&self) -> LocalBoxFuture<'_, GateLease>;

    /// Holds off every change of the node's children while an edit is
    /// prepared and committed (`message-editing.md` § Admission); refused
    /// while a child lives, starts or closes, or its completion awaits
    /// delivery. Dropping the reservation ends the hold.
    fn reserve_edit(&self) -> LocalBoxFuture<'_, Result<Option<Reservation>, String>>;

    /// The system prompt of a request that infers with `model`, whose
    /// last line names that model (`system-prompt.md` § Model identity);
    /// why it could not be rendered, such as a provider entry that is gone.
    fn system_prompt<'a>(
        &'a self,
        model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<String, String>>;

    /// The window the session's token thresholds use for `model`: the
    /// model's context window, or the limit the user set on it
    /// (`models.md` § Context limit). Asked at each check, so a changed limit
    /// counts from the next one.
    fn context_window<'a>(&'a self, model: &'a ModelSelection) -> LocalBoxFuture<'a, u32>;

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

    /// Whether consecutive calls of `tool` in one round run together as one
    /// step (`runtime.md` § Dispatch and failures); a call of any other tool
    /// is a step of its own.
    fn runs_together(&self, tool: &str) -> bool {
        let _ = tool;
        false
    }

    /// Runs one step: its calls, each of a tool that [`tools`](Self::tools)
    /// names, together. Answers each call's outcome in the order of the
    /// calls. Dropping the future stops every call.
    fn invoke_step(
        &self,
        calls: Vec<ToolInvocation>,
    ) -> LocalBoxFuture<'_, Vec<Result<ToolOutcome, ToolFailure>>>;

    /// Resolves once the first of `commands`, which a `yield` named, has
    /// ended, at once when one has ended already, with how it ended
    /// (`runtime.md` § Yield wakeups). It holds nothing of the session. A
    /// node without shells has no commands, so by default it never resolves.
    fn command_end(&self, commands: Vec<CommandId>) -> LocalBoxFuture<'static, WakeupCommand> {
        let _ = commands;
        Box::pin(std::future::pending())
    }

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
    /// Cancelled when the action stops: a command the call started stops
    /// with it, even after the call returned.
    pub cancel: CancellationToken,
    /// Resolves once input arrives that joins the turn at its next boundary,
    /// which ends the window a shell tool watches its command in
    /// (`runtime.md` § The window).
    pub arrival: InputArrival,
}

/// The arrival of input that joins the running turn at its next boundary:
/// a steer, an agent message or a fired wakeup (`runtime.md` § The window).
/// A message sent to the queue is none.
#[derive(Debug, Clone)]
pub struct InputArrival {
    arrivals: watch::Receiver<u64>,
    /// How many had arrived when the step started.
    since: u64,
}

impl InputArrival {
    /// The arrivals after the `since`th that `arrivals` counts.
    pub(crate) fn new(arrivals: watch::Receiver<u64>, since: u64) -> Self {
        Self { arrivals, since }
    }

    /// Resolves once input arrived after the step started.
    pub async fn arrived(mut self) {
        let since = self.since;
        if self.arrivals.wait_for(|count| *count > since).await.is_err() {
            // The session is gone, and no input will arrive.
            std::future::pending::<()>().await;
        }
    }
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolEffect {
    /// `yield`: schedule one wakeup `duration_ms` after the action ends, or
    /// sooner when one of `commands` ends, and end the turn after this round
    /// of tools unless input arrived during it. The call's result says
    /// `yield scheduled` with the duration and the commands.
    ScheduleYield {
        duration_ms: u32,
        commands: Vec<CommandId>,
    },
}

/// A tool that failed; its call completes as `Tool failed: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure(pub String);
