//! What a session calls in its node (`runtime.md` § Sessions and turns): the
//! admission of an action, the prompts and context, and the tools. The node
//! assembly implements it, so a session never reaches its node otherwise, and
//! a tool never reaches into its session: it returns its outcome.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

use demi_provider_common::{RequestLimits, ResultPart, ToolDefinition};
use demi_shared_gates::{GateLease, Reservation};
use demi_shared_types::{
    CommandId, CommandReport, ErrorDevice, InstructionEntry, ModelSelection, ShellToolView,
    ToolView, TurnId,
};
use futures_util::{future::LocalBoxFuture, stream::LocalBoxStream};
use serde_json::Value;
use tokio::sync::watch;

/// A system prompt by its layers (`system-prompt.md` § The layers): a
/// request carries their texts joined, and the context card weighs the
/// capability index apart from the rest (`compaction.md` § What fills the
/// context).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemPrompt {
    /// The layers in their order.
    pub layers: Vec<PromptLayer>,
}

/// One layer of a system prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptLayer {
    Text(String),
    /// The capability index, as the prompt holds it with its heading.
    Commands(String),
}

impl PromptLayer {
    /// The layer's text as the prompt holds it.
    pub fn text(&self) -> &str {
        match self {
            Self::Text(text) | Self::Commands(text) => text,
        }
    }
}

impl SystemPrompt {
    /// A prompt of one layer, as a test's runtime gives.
    pub fn identity(text: impl Into<String>) -> Self {
        Self {
            layers: vec![PromptLayer::Text(text.into())],
        }
    }

    /// The text a request carries: the layers, a blank line between two.
    pub fn text(&self) -> String {
        let texts: Vec<&str> = self.layers.iter().map(PromptLayer::text).collect();
        texts.join("\n\n")
    }
}

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
    ) -> LocalBoxFuture<'a, Result<SystemPrompt, String>>;

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

    /// What the node has running now, which a compaction pass names after
    /// its marker (`compaction.md` § What the model receives afterward): its
    /// commands that run and its live subagents. Nothing by default.
    fn running_work(&self) -> LocalBoxFuture<'_, RunningWork> {
        Box::pin(async { RunningWork::default() })
    }

    /// Whether the user recovers the node's unfinished turns, a root's, or
    /// its parent decides for it, a child's (`failures-and-recovery.md`
    /// § The unfinished turn): a root's turn left undone because a call's
    /// Host was offline ends unfinished, with an error block that offers
    /// Resume, and a root's failed turn holds its waiting input until the
    /// user acts; a child's result says what it could not do.
    fn user_recovers_turns(&self) -> bool {
        false
    }

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
    /// node's shells do not hold it and the conversation does not record it
    /// running; one it records running is taken up first
    /// (`runtime.md` § Command reports). It holds nothing of the session. A
    /// node without shells has no commands, so by default it never
    /// resolves.
    fn command_ended<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, ()> {
        let _ = command;
        Box::pin(std::future::pending())
    }

    /// What `command`, which a `shell` call left running and which reports
    /// every `interval_ms` while it runs, tells the node now (`runtime.md`
    /// § Command reports), named by the title its record keeps: its
    /// progress while it runs, which moves the node's place in its output
    /// as a look does, or its end, with the media its job viewed as a
    /// result for `model`, whose vendor takes requests within `limits`,
    /// attaches them. None when it has nothing to tell: the node saw its
    /// end in a result already and its job viewed no media, or the node
    /// stopped it itself.
    fn report<'a>(
        &'a self,
        command: &'a CommandId,
        interval_ms: Option<u32>,
        model: &'a ModelSelection,
        limits: RequestLimits,
    ) -> LocalBoxFuture<'a, Option<CommandReport>> {
        let _ = (command, interval_ms, model, limits);
        Box::pin(async { None })
    }

    /// Whether a look showed the node `command`'s end: a `demi shell status`
    /// or a `shell` result that reported it (`runtime.md` § Command
    /// reports). None by default.
    fn end_seen(&self, command: &CommandId) -> bool {
        let _ = command;
        false
    }

    /// A look at `command` as the call that watched it shows it when
    /// something other than its tool ended the call (`runtime.md`
    /// § Interrupted calls): its output since the node's last look, as a
    /// result shows it within `budget` characters, which moves the node's
    /// place in it, and the user's view of the call. None when the node's
    /// shells do not hold the command, and by default.
    fn look(&self, command: &CommandId, budget: usize) -> Option<CommandLook> {
        let _ = (command, budget);
        None
    }

    /// Whether a look of the node at `command` runs now, such as a `demi
    /// shell status --wait` that waits for its end: the command's end report
    /// then ends no window, since the look shows the end itself
    /// (`runtime.md` § Command reports). None by default.
    fn looks_at(&self, command: &CommandId) -> bool {
        let _ = command;
        false
    }

    /// Readies `report` as it is written into the transcript: it reads the
    /// output the report carries, or marks the place what it read, so that a
    /// report dropped before then moves no place in its command's output
    /// (`runtime.md` § Command reports). Nothing by default.
    fn write_report(&self, report: &mut CommandReport) {
        let _ = report;
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

/// What a node has running (`compaction.md` § What the model receives
/// afterward).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunningWork {
    /// Each command of the node that runs, by its number, with the title of
    /// the call that started it, in the order the commands started.
    pub commands: Vec<(CommandId, String)>,
    /// Each live subagent the node spawned, by its agent number, with its
    /// description, in spawn order.
    pub subagents: Vec<(u64, String)>,
}

/// A context block the model receives, as its source sees it.
#[derive(Debug, Clone, Copy)]
pub struct SeenContext<'a> {
    pub source: &'a str,
    pub text: &'a str,
}

/// What one context source answered: a new block's source and text, for
/// the instructions source what the text holds, and for the working
/// directory source the directory it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewContext {
    pub source: String,
    pub text: String,
    pub instructions: Vec<InstructionEntry>,
    pub cwd: Option<String>,
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
    /// Where the call tells its session the command it started, and learns
    /// whether something other than its tool that ends it leaves the
    /// command running (`runtime.md` § Interrupted calls).
    pub command: CallCommand,
}

/// The command a running call started, as its session knows it: what the
/// session names when something other than the tool ends the call, and
/// whether that end leaves the command running, as the backend's shutdown
/// does (`runtime.md` § Interrupted calls).
#[derive(Debug, Clone, Default)]
pub struct CallCommand(Rc<CallCommandState>);

#[derive(Debug, Default)]
struct CallCommandState {
    started: RefCell<Option<StartedCommand>>,
    keep: Cell<bool>,
}

/// A command a call started: its number and its interval. Its reports
/// name it by the title its record keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedCommand {
    pub command: CommandId,
    pub interval_ms: Option<u32>,
}

/// What a call that something other than its tool ended shows of the
/// command it watched (`runtime.md` § Interrupted calls).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLook {
    /// Its output since the node's last look, as a result shows it.
    pub output: String,
    /// The user's view of the call, which names the command.
    pub view: ShellToolView,
}

impl CallCommand {
    /// The call started `started`.
    pub fn started(&self, started: StartedCommand) {
        self.0.started.replace(Some(started));
    }

    /// The command the call started, once it did.
    pub fn command(&self) -> Option<StartedCommand> {
        self.0.started.borrow().clone()
    }

    /// Whether the call ends with the backend's shutdown, which leaves its
    /// command running.
    pub fn keeps_running(&self) -> bool {
        self.0.keep.get()
    }

    pub(crate) fn keep_running(&self) {
        self.0.keep.set(true);
    }
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
    /// only its end when none (`runtime.md` § Command reports).
    Background {
        command: CommandId,
        interval_ms: Option<u32>,
    },
    /// The call ran nothing because its Host, `device`, was offline: unless
    /// a later call of the turn runs there, a root's turn that ends after it
    /// is unfinished (`failures-and-recovery.md` § The unfinished turn).
    HostOffline(ErrorDevice),
}

/// A tool that failed; its call completes as `Tool failed: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure(pub String);
