//! What a session calls in its node (`runtime.md` § Sessions and turns): the
//! admission of an action, the prompts and context, and the tools. The node
//! assembly implements it, so a session never reaches its node otherwise, and
//! a tool never reaches into its session: it returns its outcome.

use std::sync::Arc;

use demi_provider_common::{RequestLimits, ResultPart, ToolDefinition};
use demi_shared_gates::{GateLease, Reservation};
use demi_shared_types::{CommandId, CommandReport, InstructionEntry, ModelSelection, ToolView, TurnId};
use futures_util::{future::LocalBoxFuture, stream::LocalBoxStream};
use serde_json::Value;
use tokio::sync::watch;

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
    /// names, together. Yields each call's outcome, by its index in
    /// `calls`, as the call returns, and ends once every call has
    /// returned. Dropping the stream stops the calls still running.
    fn invoke_step(&self, calls: Vec<ToolInvocation>) -> StepOutcomes<'_>;

    /// Resolves once `command`, which a `shell` call of the node left
    /// running, has ended, and at once when it has ended already or the
    /// node's shells do not hold it (`runtime.md` § Command reports). It
    /// holds nothing of the session. A node without shells has no commands,
    /// so by default it never resolves.
    fn command_ended(&self, command: &CommandId) -> LocalBoxFuture<'static, ()> {
        let _ = command;
        Box::pin(std::future::pending())
    }

    /// What `command`, which a `shell` call titled `title` left running and
    /// which reports every `interval_ms` while it runs, tells the node now
    /// (`runtime.md` § Command reports): its progress while it runs, which
    /// moves the node's place in its output as a look does, or its end. None
    /// when it has nothing to tell: the node saw its end in a result
    /// already, or stopped it itself.
    fn report<'a>(
        &'a self,
        command: &'a CommandId,
        title: &'a str,
        interval_ms: Option<u32>,
    ) -> LocalBoxFuture<'a, Option<CommandReport>> {
        let _ = (command, title, interval_ms);
        Box::pin(async { None })
    }

    /// Whether a look showed the node `command`'s end: a `demi shell status`
    /// or a `shell` result that reported it (`runtime.md` § Command
    /// reports). None by default.
    fn end_seen(&self, command: &CommandId) -> bool {
        let _ = command;
        false
    }

    /// Releases what the node's tools hold, such as its shell environments
    /// and the commands they run, once its session is disposed; the session's
    /// dispose finishes only after it. Nothing by default.
    fn dispose(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// The outcomes of a step's calls as they return, each by the call's index
/// in the step.
pub type StepOutcomes<'a> = LocalBoxStream<'a, (usize, Result<ToolOutcome, ToolFailure>)>;

/// A context block the model receives, as its source sees it.
#[derive(Debug, Clone, Copy)]
pub struct SeenContext<'a> {
    pub source: &'a str,
    pub text: &'a str,
}

/// What one context source answered: a new block's source and text, and
/// for the instructions source what the text holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewContext {
    pub source: String,
    pub text: String,
    pub instructions: Vec<InstructionEntry>,
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
    /// What ends the window a shell tool watches its command in before its
    /// time passes (`runtime.md` § The window).
    pub arrival: InputArrival,
}

/// What a session tells the windows of its running calls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Arrivals {
    /// How many agent messages and command reports arrived: input that
    /// joins the turn at its next boundary and ends a window. A human steer and a
    /// queued message are none.
    pub(crate) joining: u64,
    /// The user sent a steer or a queued message now, and the running turn
    /// has not reached its boundary yet (`runtime.md` § Send now).
    pub(crate) send_now: bool,
}

/// Why a window ended before its time passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEnd {
    /// An agent message or a command report arrived.
    Input,
    /// The user sent a message now: the command moves to the background,
    /// and the result says so.
    SentNow,
}

/// The arrival of what ends a window (`runtime.md` § The window): an agent
/// message or a command report, which joins the running turn at its next
/// boundary, or the user's send now.
#[derive(Debug, Clone)]
pub struct InputArrival {
    arrivals: watch::Receiver<Arrivals>,
    /// How many joining inputs had arrived when the step started.
    since: u64,
}

impl InputArrival {
    /// The arrivals after the `since`th joining input that `arrivals`
    /// counts.
    pub(crate) fn new(arrivals: watch::Receiver<Arrivals>, since: u64) -> Self {
        Self { arrivals, since }
    }

    /// Resolves once input arrived after the step started, or the user
    /// sent a message now, and says which.
    pub async fn arrived(mut self) -> WindowEnd {
        let since = self.since;
        match self
            .arrivals
            .wait_for(|arrivals| arrivals.send_now || arrivals.joining > since)
            .await
        {
            Ok(arrivals) if arrivals.send_now => WindowEnd::SentNow,
            Ok(_) => WindowEnd::Input,
            // The session is gone, and nothing will arrive.
            Err(_) => std::future::pending().await,
        }
    }

    /// Whether the user sent a message now: a call that has not started
    /// then never starts, and completes as [`ToolOutcome::not_run`].
    pub fn sent_now(&self) -> bool {
        self.arrivals.borrow().send_now
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
    /// What the session does beyond recording the result.
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

    /// A requested call that had not started when the user sent a message
    /// now (`runtime.md` § Send now).
    pub fn not_run() -> Self {
        Self::error("Tool call not run: the user sent a message".to_owned())
    }
}

/// What a tool asks of its session beside its result (`runtime.md`
/// § Dispatch and failures): a tool never reaches into its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolEffect {
    /// The call returned while its command still runs: the session watches
    /// it, and the command reports every `interval_ms` until it ends, or
    /// only its end when none (`runtime.md` § Command reports). Its reports
    /// name it by `title`, the call's.
    Background {
        command: CommandId,
        interval_ms: Option<u32>,
        title: String,
    },
}

/// A tool that failed; its call completes as `Tool failed: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure(pub String);
