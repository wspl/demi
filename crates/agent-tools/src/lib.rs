//! The standard tools (`runtime.md` § Tools): `shell_exec`, `shell_status`,
//! `shell_write`, `shell_abort` and `yield`, and only these. The shell tools
//! reach the conversation's current Host through the product's resolver and
//! run in the node's shell environment for that Host, which the product
//! makes; `yield` returns an effect for the session to apply. A tool never
//! reaches into its session: it returns its outcome. The product also
//! supplies the context sources, and the rules of these tools open every
//! node's system prompt.

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
use demi_agent_session::{ToolEffect, ToolFailure, ToolInvocation, ToolOutcome};
use demi_agent_store::AgentTreeStore;
use demi_host_interface::{
    CommandSet, CommandState, CommandStatus, ExecRequest, Host, HostError, JobCaller, Numbers,
    ObservationWindow, PageFeed, ShellEnvironment, ShellError, ShellTarget,
};
use demi_provider_common::{RequestLimits, ToolDefinition};
use demi_shared_types::{CommandId, ModelSelection, NodeId, Sequence};
use futures_util::future::LocalBoxFuture;

pub use environments::Environments;
use environments::Handle;
pub use frames::{shell_output, stored_running_commands};
use input::{CommandInput, ShellExecInput, ShellWriteInput, YieldInput, parse};
pub use product::{
    ContextSource, HostResolver, NodeContext, Profile, ProfileModel, SubagentSettings,
    SubagentSource, Toolset, ToolsetSource, Unavailable,
};
pub use prompt::system_prompt;

/// The most characters a page of `demi shell output` takes, so that a tool
/// result printing one is never cut.
pub const PAGE_CHARS: usize = 12_000;

/// The conversation's command and shell numbers, as its tree store gives
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
    /// The commands the environment's shells offer: the node's.
    pub commands: &'a Rc<CommandSet>,
    /// Where the environment tells the pages of its commands, and learns
    /// whether a page watches: the node's feed of its tree
    /// (`runtime.md` § Live output).
    pub feed: &'a Rc<dyn PageFeed>,
    /// Where the environment's command and shell numbers come from: the
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

/// The five tools, in the order the model is given them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StandardTool {
    ShellExec,
    ShellStatus,
    ShellWrite,
    ShellAbort,
    Yield,
}

impl StandardTool {
    const ALL: [Self; 5] = [
        Self::ShellExec,
        Self::ShellStatus,
        Self::ShellWrite,
        Self::ShellAbort,
        Self::Yield,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::ShellExec => "shell_exec",
            Self::ShellStatus => "shell_status",
            Self::ShellWrite => "shell_write",
            Self::ShellAbort => "shell_abort",
            Self::Yield => "yield",
        }
    }

    fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.name() == name)
    }

    fn definition(self) -> ToolDefinition {
        let (description, input_schema) = match self {
            Self::ShellExec => (
                "Start a shell script and observe it for up to timeoutMs. timeoutMs is an observation window, not a kill deadline: at timeoutMs the command keeps running and a command handle (commandId) is returned. Completed short output is returned directly. shell_exec never ends the turn or schedules a wakeup on its own.",
                input::schema::<ShellExecInput>(),
            ),
            Self::ShellStatus => (
                "Read a running command handle status and any new budgeted output preview. Does not wait or write stdin.",
                input::schema::<CommandInput>(),
            ),
            Self::ShellWrite => (
                "Write non-empty stdin to a running foreground command and return status with new budgeted output preview. Include a newline for line-oriented prompts.",
                input::schema::<ShellWriteInput>(),
            ),
            Self::ShellAbort => (
                "Stop a running foreground command by commandId.",
                input::schema::<CommandInput>(),
            ),
            Self::Yield => (
                "End this turn and schedule a one-shot wakeup. Does not touch shell commands.",
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

/// A node's access to its shells: the resolver that names the current Host,
/// the product's factory, the environments made so far, and the node they
/// belong to.
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
}

impl<H: HostResolver> ShellAccess<'_, H> {
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

    /// Runs one call of a standard tool. A refused input completes the call
    /// as an error that names the offending field; a failure of the Host or
    /// its shells completes it as `Tool failed: <message>`.
    pub async fn invoke(&self, call: ToolInvocation) -> Result<ToolOutcome, ToolFailure> {
        let Some(tool) = StandardTool::named(&call.tool_name) else {
            return Ok(ToolOutcome::error(format!(
                "Tool not found: {}",
                call.tool_name
            )));
        };
        match self.run(tool, call).await {
            Ok(outcome) => Ok(outcome),
            Err(CallError::Refused(text)) => Ok(ToolOutcome::error(text)),
            Err(CallError::Failed(message)) => Err(ToolFailure(message)),
        }
    }

    async fn run(
        &self,
        tool: StandardTool,
        call: ToolInvocation,
    ) -> Result<ToolOutcome, CallError> {
        let name = tool.name();
        let ToolInvocation {
            tool_use_id,
            input,
            model,
            request_limits,
            cancel,
            ..
        } = call;
        let called = Called {
            model: &model,
            limits: request_limits,
        };
        Ok(match tool {
            StandardTool::Yield => {
                let input: YieldInput = parse(name, input).map_err(CallError::Refused)?;
                // The session writes the result: the wakeup's id is its own.
                ToolOutcome {
                    output: Vec::new(),
                    is_error: false,
                    view: None,
                    effect: Some(ToolEffect::ScheduleYield {
                        duration_ms: input.duration_ms.0,
                    }),
                }
            }
            StandardTool::ShellExec => {
                let input: ShellExecInput = parse(name, input).map_err(CallError::Refused)?;
                let handle = input.shell_id.as_ref().map_or(Handle::None, Handle::Shell);
                let (slot, environment) = self.environment(handle).await?;
                if let Some(suppressed) = slot.repeated(&input.script) {
                    return Ok(suppressed);
                }
                let request = ExecRequest {
                    script: input.script,
                    shell: input
                        .shell_id
                        .map_or(ShellTarget::Default, ShellTarget::Existing),
                    window: ObservationWindow::from_millis(u64::from(input.timeout_ms.0))
                        .expect("the input admits only windows an environment takes"),
                    caller: JobCaller {
                        node: self.context.node.clone(),
                    },
                    tool_use_id,
                };
                let status = environment.exec(request, cancel).await?;
                finish(environment.as_ref(), status, called).await
            }
            StandardTool::ShellStatus => {
                let input: CommandInput = parse(name, input).map_err(CallError::Refused)?;
                let (_, environment) = self.environment(Handle::Command(&input.command_id)).await?;
                let status = environment.status(&input.command_id)?;
                finish(environment.as_ref(), status, called).await
            }
            StandardTool::ShellWrite => {
                let input: ShellWriteInput = parse(name, input).map_err(CallError::Refused)?;
                let environment = self.write(&input.command_id, input.stdin.0).await?;
                let status = environment.status(&input.command_id)?;
                finish(environment.as_ref(), status, called).await
            }
            StandardTool::ShellAbort => {
                let input: CommandInput = parse(name, input).map_err(CallError::Refused)?;
                let environment = self.abort(&input.command_id).await?;
                let status = environment.status(&input.command_id)?;
                // A stop the model asked for is never an error.
                ToolOutcome {
                    is_error: false,
                    ..finish(environment.as_ref(), status, called).await
                }
            }
        })
    }

    /// Writes `stdin` to a running command of the current Host, and returns
    /// the environment that runs it.
    pub async fn write(
        &self,
        command: &CommandId,
        stdin: String,
    ) -> Result<Rc<dyn ShellEnvironment>, CallError> {
        let (_, environment) = self.environment(Handle::Command(command)).await?;
        environment.write(command, Bytes::from(stdin)).await?;
        Ok(environment)
    }

    /// Stops a running command of the current Host, and returns the
    /// environment that ran it.
    pub async fn abort(&self, command: &CommandId) -> Result<Rc<dyn ShellEnvironment>, CallError> {
        let (_, environment) = self.environment(Handle::Command(command)).await?;
        environment.abort(command).await?;
        Ok(environment)
    }
}

/// Why a call did not produce a result of its tool.
#[derive(Debug)]
pub enum CallError {
    /// The input is refused, with the text the model receives.
    Refused(String),
    /// The Host or its shells failed.
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

/// The model of the request that asked for a call, and what its vendor
/// takes in one request.
#[derive(Clone, Copy)]
struct Called<'a> {
    model: &'a ModelSelection,
    limits: RequestLimits,
}

/// A shell tool's outcome. A result that reports the command's end releases
/// its handle; `demi shell output` reads its output from then on.
async fn finish(
    environment: &dyn ShellEnvironment,
    status: CommandStatus,
    called: Called<'_>,
) -> ToolOutcome {
    let outcome = result::shell_outcome(&status, &called.model.model, called.limits).await;
    if !matches!(status.state, CommandState::Running { .. }) {
        // A command the environment already forgot has nothing to release.
        environment.release_command(&status.command_id).await;
    }
    outcome
}
