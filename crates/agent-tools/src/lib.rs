//! The standard tools (`runtime.md` § Tools): `shell_exec`, `shell_status`
//! and `yield`, and only these. The shell tools
//! reach the conversation's current Host through the product's resolver and
//! run in the node's shell environment for that Host, which the product
//! makes; `yield` returns an effect for the session to apply. A tool never
//! reaches into its session: it returns its outcome. The product also
//! supplies the context sources, and the rules of these tools are a layer
//! of every node's system prompt.

mod environments;
mod frames;
mod input;
mod product;
mod prompt;
mod result;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

use std::{
    rc::Rc,
    sync::{Arc, LazyLock},
};

use bytes::Bytes;
use demi_agent_session::{
    StepOutcomes, ToolEffect, ToolFailure, ToolInvocation, ToolOutcome, WindowEnd,
};
use demi_agent_store::AgentTreeStore;
use demi_host_interface::{
    CommandSet, CommandState, CommandStatus, ExecRequest, Host, HostError, JobCaller,
    Numbers, ObservationWindow, PageFeed, ShellEnvironment, ShellError, watch,
};
use demi_provider_common::{RequestLimits, ToolDefinition};
use demi_shared_types::{CommandId, ModelSelection, NodeId, Sequence};
use futures_util::{
    StreamExt,
    future::LocalBoxFuture,
    stream::{self, FuturesUnordered},
};

pub use environments::Environments;
use environments::Handle;
pub use frames::{shell_output, stored_running_commands};
use input::{DelayMs, ShellExecInput, StatusFields, StatusInput, YieldInput, parse};
pub use product::{
    ContextAnswer, ContextSource, HostResolver, NodeContext, Profile, ProfileModel, SubagentSettings,
    SubagentSource, Toolset, ToolsetSource, Unavailable,
};
pub use prompt::{ModelIdentity, system_prompt};

/// The most characters a page of `demi shell output` takes, so that a tool
/// result printing one is never cut.
pub const PAGE_CHARS: usize = 12_000;

/// The conversation's command numbers, as its tree store gives
/// them out.
pub struct StoreNumbers(pub Rc<dyn AgentTreeStore>);

impl Numbers for StoreNumbers {
    fn next(&self, sequence: Sequence) -> LocalBoxFuture<'_, Result<u64, HostError>> {
        Box::pin(async move {
            self.0
                .next_number(sequence)
                .await
                .map_err(|error| HostError::failed(None, error.to_string()))
        })
    }
}

/// The node an environment is made for.
#[derive(Clone, Copy)]
pub struct EnvironmentScope<'a> {
    /// The conversation's root node.
    pub root: &'a NodeId,
    pub node: &'a NodeId,
    /// The node's agent number, as the model and its commands' context know
    /// it (`runtime.md` § Identifiers the model sees).
    pub agent: u64,
    /// The commands the environment's jobs offer: the node's.
    pub commands: &'a Rc<CommandSet>,
    /// Where the environment tells the pages of its commands, and learns
    /// whether a page watches: the node's feed of its tree
    /// (`runtime.md` § Live output).
    pub feed: &'a Rc<dyn PageFeed>,
    /// Where the environment's command numbers come from: the
    /// conversation's sequences, which the tree store gives out.
    pub numbers: &'a Rc<dyn Numbers>,
}

/// Where a node's shell environments come from. The product makes the
/// environment of one node on one of its Hosts, such as backend-remote-host's over
/// the Host's runner; the agent never knows which shell engine runs.
pub trait ShellEnvironmentFactory<H> {
    fn create<'a>(
        &'a self,
        scope: EnvironmentScope<'a>,
        host: Rc<H>,
    ) -> LocalBoxFuture<'a, Result<Rc<dyn ShellEnvironment>, HostError>>;
}

/// The three tools, in the order the model is given them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StandardTool {
    ShellExec,
    ShellStatus,
    Yield,
}

impl StandardTool {
    const ALL: [Self; 3] = [Self::ShellExec, Self::ShellStatus, Self::Yield];

    fn name(self) -> &'static str {
        match self {
            Self::ShellExec => "shell_exec",
            Self::ShellStatus => "shell_status",
            Self::Yield => "yield",
        }
    }

    fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.name() == name)
    }

    fn definition(self) -> ToolDefinition {
        let (description, input_schema) = match self {
            Self::ShellExec => (
                "Start a shell script and watch it for up to timeoutMs. timeoutMs is how long to watch, not a deadline: when it passes, or when the user steers, the command keeps running and the result carries its commandId. Completed short output is returned directly. The shell_exec calls of one response run at the same time.",
                input::schema::<ShellExecInput>(),
            ),
            Self::ShellStatus => (
                "Look at a command by its commandId: write stdin to it first when given (description is then required), then watch it for up to timeoutMs, or look at once without timeoutMs. Returns its status and the output since your last look.",
                input::schema::<StatusFields>(),
            ),
            Self::Yield => (
                "End this turn and be woken after durationMs, or as soon as the first of the commands commandIds names ends, whichever comes first. The user can talk to you meanwhile.",
                input::schema::<YieldInput>(),
            ),
        };
        ToolDefinition {
            name: self.name().to_owned(),
            description: description.to_owned(),
            input_schema,
        }
    }
}

/// The definitions the model receives, made once.
pub fn definitions() -> Arc<[ToolDefinition]> {
    static DEFINITIONS: LazyLock<Arc<[ToolDefinition]>> =
        LazyLock::new(|| StandardTool::ALL.map(StandardTool::definition).into());
    DEFINITIONS.clone()
}

/// Whether consecutive calls of `tool` in one round run together as one
/// step: `shell_exec`'s do (`runtime.md` § Dispatch and failures).
pub fn runs_together(tool: &str) -> bool {
    StandardTool::named(tool) == Some(StandardTool::ShellExec)
}

/// The conversation's commands, whichever agent of it ran them, which a
/// `yield` may name (`runtime.md` § Yield wakeups).
pub trait ConversationCommands {
    /// Whether `command` is a command of the conversation, running or
    /// ended.
    fn knows<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<bool, String>>;
}

/// A node's access to its shell environments: the resolver that names the
/// current Host, the product's factory, the environments made so far, and
/// the node they belong to.
pub struct ShellAccess<'a, H: HostResolver> {
    pub hosts: &'a H,
    pub shells: &'a dyn ShellEnvironmentFactory<H::Host>,
    pub environments: &'a Environments,
    pub context: NodeContext<'a>,
    /// The node's agent number.
    pub agent: u64,
    pub commands: &'a Rc<CommandSet>,
    /// The node's feed, which its environments are made with.
    pub feed: &'a Rc<dyn PageFeed>,
    /// The conversation's numbers, which its environments are made with.
    pub numbers: &'a Rc<dyn Numbers>,
    /// The conversation's commands, which a `yield` names.
    pub conversation: &'a dyn ConversationCommands,
}

// Every field is a reference or a number, so a step's tasks each hold a
// copy; derived, the copy would ask the same of the resolver `H`.
impl<H: HostResolver> Clone for ShellAccess<'_, H> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<H: HostResolver> Copy for ShellAccess<'_, H> {}

impl<'a, H: HostResolver> ShellAccess<'a, H> {
    /// The environment for the conversation's current Host, which `handle`
    /// must belong to.
    async fn environment(
        &self,
        handle: Handle<'_>,
    ) -> Result<(Rc<environments::Slot>, Rc<dyn ShellEnvironment>), CallError> {
        let host = self
            .hosts
            .host(self.context)
            .await
            .map_err(|error| CallError::Failed(error.to_string()))?;
        let scope = EnvironmentScope {
            root: self.context.root,
            node: self.context.node,
            agent: self.agent,
            commands: self.commands,
            feed: self.feed,
            numbers: self.numbers,
        };
        let key = host.key();
        self.environments
            .resolve(key, handle, self.shells.create(scope, host))
            .await
            .map_err(CallError::Failed)
    }

    /// Runs one step of a round (`runtime.md` § Dispatch and failures): a
    /// call of `shell_status` or `yield`, or `shell_exec` calls, which start
    /// together. Yields each call's outcome, by its index, as the call
    /// returns. A refused input completes its call as an error that names
    /// the offending field; a failure of the Host or its environment
    /// completes it as `Tool failed: <message>`.
    pub fn invoke_step(self, calls: Vec<ToolInvocation>) -> StepOutcomes<'a> {
        if !calls.is_empty() && calls.iter().all(|call| runs_together(&call.tool_name)) {
            return Box::pin(stream::once(self.exec_step(calls)).flatten());
        }
        let calls: FuturesUnordered<_> = calls
            .into_iter()
            .enumerate()
            .map(|(index, call)| async move { (index, self.invoke(call).await) })
            .collect();
        Box::pin(calls)
    }

    /// Runs one call of a standard tool.
    async fn invoke(self, call: ToolInvocation) -> Result<ToolOutcome, ToolFailure> {
        let Some(tool) = StandardTool::named(&call.tool_name) else {
            return Ok(ToolOutcome::error(format!(
                "Tool not found: {}",
                call.tool_name
            )));
        };
        match tool {
            StandardTool::ShellExec => {
                let mut ran = self.exec_step(vec![call]).await;
                ran.next().await.expect("one call").1
            }
            StandardTool::ShellStatus => outcome(self.status(call).await),
            StandardTool::Yield => outcome(self.yield_wakeup(call).await),
        }
    }

    /// Runs a step's `shell_exec` calls together, each a job of its own
    /// (`runtime.md` § Dispatch and failures), yielding each call's outcome
    /// as it returns.
    async fn exec_step(self, calls: Vec<ToolInvocation>) -> StepOutcomes<'a> {
        let mut refused = Vec::new();
        let mut planned = Vec::with_capacity(calls.len());
        for (index, call) in calls.into_iter().enumerate() {
            match parse::<ShellExecInput>(StandardTool::ShellExec.name(), call.input.clone()) {
                Ok(input) => planned.push((index, call, input)),
                Err(refusal) => refused.push((index, Ok(ToolOutcome::error(refusal)))),
            }
        }
        let execs: FuturesUnordered<_> = planned
            .into_iter()
            .map(|(index, call, input)| async move { (index, outcome(self.exec(call, input).await)) })
            .collect();
        Box::pin(stream::iter(refused).chain(execs))
    }

    /// Starts a `shell_exec` call's script and watches it for up to its
    /// window (`runtime.md` § The window).
    async fn exec(
        &self,
        call: ToolInvocation,
        input: ShellExecInput,
    ) -> Result<ToolOutcome, CallError> {
        // A call after one that the user's send now returned never starts
        // (`runtime.md` § Send now).
        if call.arrival.sent_now() {
            return Ok(ToolOutcome::not_run());
        }
        let (slot, environment) = self.environment(Handle::None).await?;
        if let Some(suppressed) = slot.repeated(&input.script) {
            return Ok(suppressed);
        }
        let request = ExecRequest {
            script: input.script,
            caller: JobCaller {
                node: self.context.node.clone(),
            },
            tool_use_id: call.tool_use_id,
        };
        let command = environment.start(request, call.cancel).await?;
        let (status, ended_by) = watch(
            environment.as_ref(),
            &command,
            Some(window(input.timeout_ms)),
            call.arrival.arrived(),
        )
        .await?;
        let called = Called {
            model: &call.model,
            limits: call.request_limits,
            sent_now: ended_by == Some(WindowEnd::SentNow),
            above_cap: watched_above_cap(Some(input.timeout_ms)),
        };
        Ok(finish(environment.as_ref(), status, called).await)
    }

    /// A `shell_status` call: writes its input to the command first when it
    /// has some, then watches the command for up to its window, or looks at
    /// once without one.
    async fn status(&self, call: ToolInvocation) -> Result<ToolOutcome, CallError> {
        let input: StatusInput =
            parse(StandardTool::ShellStatus.name(), call.input).map_err(CallError::Refused)?;
        if call.arrival.sent_now() {
            return Ok(ToolOutcome::not_run());
        }
        let command = &input.command_id;
        let (_, environment) = self.environment(Handle::Command(command)).await?;
        if let Some(stdin) = input.stdin {
            environment.write(command, Bytes::from(stdin.0)).await?;
        }
        let (status, ended_by) = watch(
            environment.as_ref(),
            command,
            input.timeout_ms.map(window),
            call.arrival.arrived(),
        )
        .await?;
        let called = Called {
            model: &call.model,
            limits: call.request_limits,
            sent_now: ended_by == Some(WindowEnd::SentNow),
            above_cap: watched_above_cap(input.timeout_ms),
        };
        Ok(finish(environment.as_ref(), status, called).await)
    }

    /// A `yield` call: the wakeup it asks the session for. A command it
    /// names must be one of the conversation's.
    async fn yield_wakeup(&self, call: ToolInvocation) -> Result<ToolOutcome, CallError> {
        let input: YieldInput =
            parse(StandardTool::Yield.name(), call.input).map_err(CallError::Refused)?;
        let commands = input.command_ids.unwrap_or_default();
        for command in &commands {
            if !self
                .conversation
                .knows(command)
                .await
                .map_err(CallError::Failed)?
            {
                return Ok(ToolOutcome::error(format!(
                    "yield: no command {command} in this conversation"
                )));
            }
        }
        // The session writes the result: the wakeup's id is its own.
        Ok(ToolOutcome {
            output: Vec::new(),
            is_error: false,
            view: None,
            effect: Some(ToolEffect::ScheduleYield {
                duration_ms: input.duration_ms.taken(),
                commands,
                above_cap: input.duration_ms.above_cap("durationMs", "scheduled for"),
            }),
        })
    }

    /// Writes `stdin` to a running command of the current Host, as a page's
    /// input does.
    pub async fn write(&self, command: &CommandId, stdin: String) -> Result<(), CallError> {
        let (_, environment) = self.environment(Handle::Command(command)).await?;
        environment.write(command, Bytes::from(stdin)).await?;
        Ok(())
    }

    /// Stops a running command of the current Host, as a page's stop does.
    pub async fn abort(&self, command: &CommandId) -> Result<(), CallError> {
        let (_, environment) = self.environment(Handle::Command(command)).await?;
        environment.abort(command).await?;
        Ok(())
    }
}

/// A call's outcome: a refused input completes it as an error the model
/// reads, a failure as `Tool failed: <message>`.
fn outcome(ran: Result<ToolOutcome, CallError>) -> Result<ToolOutcome, ToolFailure> {
    match ran {
        Ok(outcome) => Ok(outcome),
        Err(CallError::Refused(text)) => Ok(ToolOutcome::error(text)),
        Err(CallError::Failed(message)) => Err(ToolFailure(message)),
    }
}

/// The window a call's `timeoutMs` names, at most the cap.
fn window(timeout: DelayMs) -> ObservationWindow {
    ObservationWindow::from_millis(u64::from(timeout.taken()))
        .expect("the cap is a window an environment takes")
}

/// The line a shell tool's result starts with when its `timeoutMs` was above
/// the cap (`runtime.md` § Tool input).
fn watched_above_cap(timeout: Option<DelayMs>) -> Option<String> {
    timeout?.above_cap("timeoutMs", "watched for")
}

/// Why a call did not produce a result of its tool.
#[derive(Debug)]
pub enum CallError {
    /// The input is refused, with the text the model receives.
    Refused(String),
    /// The Host or its environment failed.
    Failed(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(text) | Self::Failed(text) => formatter.write_str(text),
        }
    }
}

impl From<ShellError> for CallError {
    fn from(error: ShellError) -> Self {
        Self::Failed(error.to_string())
    }
}

/// The model of the request that asked for a call, what its vendor takes in
/// one request, whether the user's send now ended the call's window, and
/// the line that says its window was above the cap.
struct Called<'a> {
    model: &'a ModelSelection,
    limits: RequestLimits,
    sent_now: bool,
    above_cap: Option<String>,
}

/// A shell tool's outcome. A result that reports the command's end releases
/// its handle; `demi shell output` reads its output from then on.
async fn finish(
    environment: &dyn ShellEnvironment,
    status: CommandStatus,
    called: Called<'_>,
) -> ToolOutcome {
    let outcome = result::shell_outcome(
        &status,
        &called.model.model,
        called.limits,
        called.sent_now,
        called.above_cap.as_deref(),
    )
    .await;
    if !matches!(status.state, CommandState::Running { .. }) {
        // A command the environment already forgot has nothing to release.
        environment.release_command(&status.command_id).await;
    }
    outcome
}
