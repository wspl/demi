//! The standard tools (`runtime.md` § Tools): `shell_exec`, `shell_status`,
//! `shell_write`, `shell_abort` and `yield`, and only these. The shell tools
//! reach the conversation's current Host through the harness and run in the
//! node's shell environment for that Host, which the product makes; `yield`
//! returns an effect for the session to apply. A tool never reaches into its
//! session: it returns its outcome.

mod environments;
mod frames;
mod input;
mod result;

use std::{
    rc::Rc,
    sync::{Arc, LazyLock},
};

use bytes::Bytes;
use demi_core::{CommandId, ModelSelection, NodeId};
use demi_provider::ToolDefinition;
use demi_shell::{
    CommandSet, CommandStatus, ExecRequest, Host, HostError, JobCaller, ObservationWindow,
    Reader, ShellEnvironment, ShellError, ShellTarget,
};
use futures_util::future::LocalBoxFuture;

pub(crate) use environments::Environments;
use environments::Handle;
pub(crate) use frames::{shell_output, stored_running_commands};
use input::{CommandInput, ShellExecInput, ShellWriteInput, YieldInput, parse};

use crate::{
    AgentHarness, PromptContext,
    session::{ToolEffect, ToolFailure, ToolInvocation, ToolOutcome},
};

/// The node an environment is made for.
#[derive(Clone, Copy)]
pub struct EnvironmentScope<'a> {
    /// The conversation's root node.
    pub root: &'a NodeId,
    pub node: &'a NodeId,
    /// The commands the environment's shells offer: the node's.
    pub commands: &'a Rc<CommandSet>,
}

/// Where a node's shell environments come from. The product makes the
/// environment of one node on one of its Hosts, such as host-remote's over
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
pub(crate) fn definitions() -> Arc<[ToolDefinition]> {
    static DEFINITIONS: LazyLock<Arc<[ToolDefinition]>> =
        LazyLock::new(|| StandardTool::ALL.map(StandardTool::definition).into());
    DEFINITIONS.clone()
}

/// A node's access to its shells: the harness that names the current Host,
/// the product's factory, the environments made so far, and the node they
/// belong to.
pub(crate) struct ShellAccess<'a, H: AgentHarness> {
    pub(crate) harness: &'a H,
    pub(crate) shells: &'a dyn ShellEnvironmentFactory<H::Host>,
    pub(crate) environments: &'a Environments,
    pub(crate) context: PromptContext<'a>,
    pub(crate) commands: &'a Rc<CommandSet>,
    /// Where a running shell tool's status goes: the root's client; a
    /// child's reaches no one.
    pub(crate) progress: Option<&'a dyn Fn(&CommandStatus)>,
}

impl<H: AgentHarness> ShellAccess<'_, H> {
    /// The environment for the conversation's current Host, which `handle`
    /// must belong to.
    async fn environment(
        &self,
        handle: Handle<'_>,
    ) -> Result<(Rc<environments::Slot>, Rc<dyn ShellEnvironment>), CallError> {
        let host = self
            .harness
            .host(self.context)
            .await
            .map_err(|error| CallError::Failed(error.to_string()))?;
        let scope = EnvironmentScope {
            root: self.context.root,
            node: self.context.node,
            commands: self.commands,
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
    pub(crate) async fn invoke(&self, call: ToolInvocation) -> Result<ToolOutcome, ToolFailure> {
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
            input,
            model,
            generation,
            cancel,
            ..
        } = call;
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
                        generation,
                    },
                };
                let status = environment.exec(request, cancel).await?;
                self.report(environment.as_ref(), &status.command_id);
                finish(environment.as_ref(), status, &model).await
            }
            StandardTool::ShellStatus => {
                let input: CommandInput = parse(name, input).map_err(CallError::Refused)?;
                let (_, environment) = self.environment(Handle::Command(&input.command_id)).await?;
                let status = environment.status(&input.command_id, Reader::Model)?;
                finish(environment.as_ref(), status, &model).await
            }
            StandardTool::ShellWrite => {
                let input: ShellWriteInput = parse(name, input).map_err(CallError::Refused)?;
                let (environment, status) = self
                    .write(&input.command_id, input.stdin.0, Reader::Model)
                    .await?;
                self.report(environment.as_ref(), &status.command_id);
                finish(environment.as_ref(), status, &model).await
            }
            StandardTool::ShellAbort => {
                let input: CommandInput = parse(name, input).map_err(CallError::Refused)?;
                let (environment, status) = self.abort(&input.command_id, Reader::Model).await?;
                self.report(environment.as_ref(), &status.command_id);
                // A stop the model asked for is never an error.
                ToolOutcome {
                    is_error: false,
                    ..finish(environment.as_ref(), status, &model).await
                }
            }
        })
    }

    /// Sends a running shell tool's command to the root's client, as the
    /// page's own view of it: the model's view is the model's. A
    /// `shell_status` call sends none.
    fn report(&self, environment: &dyn ShellEnvironment, command: &CommandId) {
        if let Some(progress) = self.progress
            && let Ok(status) = environment.status(command, Reader::Page)
        {
            progress(&status);
        }
    }

    /// The page's status of `command`, when an environment of the node owns it.
    pub(crate) fn status_of(&self, command: &CommandId) -> Option<CommandStatus> {
        // The environment that owns the command answers its status: nothing
        // happens between the two, so it cannot forget the command meanwhile.
        self.environments
            .owning(command)?
            .status(command, Reader::Page)
            .ok()
    }

    /// Writes `stdin` to a running command of the current Host; the status is
    /// `reader`'s.
    pub(crate) async fn write(
        &self,
        command: &CommandId,
        stdin: String,
        reader: Reader,
    ) -> Result<(Rc<dyn ShellEnvironment>, CommandStatus), CallError> {
        let (_, environment) = self.environment(Handle::Command(command)).await?;
        let status = environment.write(command, Bytes::from(stdin), reader).await?;
        Ok((environment, status))
    }

    /// Stops a running command of the current Host; the status is
    /// `reader`'s.
    pub(crate) async fn abort(
        &self,
        command: &CommandId,
        reader: Reader,
    ) -> Result<(Rc<dyn ShellEnvironment>, CommandStatus), CallError> {
        let (_, environment) = self.environment(Handle::Command(command)).await?;
        let status = environment.abort(command, reader).await?;
        Ok((environment, status))
    }
}

/// Why a call did not produce a result of its tool.
#[derive(Debug)]
pub(crate) enum CallError {
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

/// A shell tool's outcome, with the preview budget of the call's model; a
/// command whose result showed everything has its handle released.
async fn finish(
    environment: &dyn ShellEnvironment,
    status: CommandStatus,
    model: &ModelSelection,
) -> ToolOutcome {
    let model = &model.model;
    let budget = result::preview_budget_tokens(model.context_window);
    let expose = result::handle_required(&status, budget);
    let outcome = result::shell_outcome(&status, budget, expose, model);
    if !expose {
        // A command the environment already forgot has nothing to release.
        environment.release_command(&status.command_id).await;
    }
    outcome
}
