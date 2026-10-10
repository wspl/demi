//! The one tool, `shell` (`runtime.md` § Tools): it reaches the
//! conversation's current Host through the product's resolver and runs in
//! the node's shell environment for that Host, which the product makes. A
//! tool never reaches into its session: it returns its outcome, and a call
//! that leaves its command running asks the session to watch it, so the
//! command reports to the node (`runtime.md` § Command reports). What the
//! `demi shell` commands and the reports show of a command is made here as
//! well. The product also supplies the context sources, and the rules of the
//! tool are a layer of every node's system prompt.

mod environments;
mod frames;
mod input;
mod product;
mod prompt;
mod reports;
mod result;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

use std::{
    rc::Rc,
    sync::{Arc, LazyLock},
    time::Duration,
};

use bytes::Bytes;
use demi_agent_session::{
    StepOutcomes, ToolEffect, ToolFailure, ToolInvocation, ToolOutcome, WindowEnd,
};
use demi_agent_store::AgentTreeStore;
use demi_host_interface::{
    CommandSet, CommandState, CommandStatus, ExecRequest, Host, HostError, JobCaller, Numbers,
    PageFeed, ShellEnvironment, ShellError,
};
use demi_provider_common::ToolDefinition;
use demi_shared_types::{CommandId, NodeId, Sequence};
use futures_util::{future::LocalBoxFuture, stream::FuturesUnordered};

pub use environments::{Environments, Stopper};
use environments::Handle;
pub use frames::{shell_output, stored_running_commands};
pub use input::{INTERVAL_CAP_MS, INTERVAL_FLOOR_MS, taken_interval};
use input::{ShellInput, parse};
pub use reports::{EndOf, end_report, progress_report};
pub use result::{Look, look_text, whole_look_text};
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

/// The tool's name.
const SHELL: &str = "shell";

/// What the `shell` tool does, as the model is told.
const SHELL_DESCRIPTION: &str = "Start a shell script in the conversation's working directory and watch it as intervalMs says. Watching is not a deadline: a command still running when the call returns keeps running, the result carries its commandId, and the command reports to you as its interval says until it ends. Completed short output is returned directly. The shell calls of one response run at the same time.";

/// The definitions the model receives, made once: the `shell` tool alone.
pub fn definitions() -> Arc<[ToolDefinition]> {
    static DEFINITIONS: LazyLock<Arc<[ToolDefinition]>> = LazyLock::new(|| {
        Arc::new([ToolDefinition {
            name: SHELL.to_owned(),
            description: SHELL_DESCRIPTION.to_owned(),
            input_schema: input::schema::<ShellInput>(),
        }])
    });
    DEFINITIONS.clone()
}

/// Whether consecutive calls of `tool` in one round run together as one
/// step: `shell`'s do (`runtime.md` § Dispatch and failures).
pub fn runs_together(tool: &str) -> bool {
    tool == SHELL
}

/// How long a resident command's call waits for its output to be quiet.
const QUIET: Duration = Duration::from_secs(2);
/// The longest a resident command's call watches it.
const RESIDENT_WINDOW: Duration = Duration::from_secs(30);

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
    /// The least interval a command reports at, in milliseconds:
    /// [`INTERVAL_FLOOR_MS`] in the product.
    pub interval_floor_ms: u32,
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

    /// Runs one step of a round (`runtime.md` § Dispatch and failures):
    /// `shell` calls, which start together, each a job of its own, or a call
    /// of a tool the node does not have, which completes as `Tool not
    /// found`. Yields each call's outcome, by its index, as the call
    /// returns. A refused input completes its call as an error that names
    /// the offending field; a failure of the Host or its environment
    /// completes it as `Tool failed: <message>`.
    pub fn invoke_step(self, calls: Vec<ToolInvocation>) -> StepOutcomes<'a> {
        let calls: FuturesUnordered<_> = calls
            .into_iter()
            .enumerate()
            .map(|(index, call)| async move { (index, self.invoke(call).await) })
            .collect();
        Box::pin(calls)
    }

    /// Runs one call.
    async fn invoke(self, call: ToolInvocation) -> Result<ToolOutcome, ToolFailure> {
        if call.tool_name != SHELL {
            return Ok(ToolOutcome::error(format!(
                "Tool not found: {}",
                call.tool_name
            )));
        }
        match parse::<ShellInput>(SHELL, call.input.clone()) {
            Ok(input) => outcome(self.shell(call, input).await),
            Err(refusal) => Ok(ToolOutcome::error(refusal)),
        }
    }

    /// Starts a `shell` call's script and watches it as its interval says
    /// (`runtime.md` § The window); a command that still runs then reports
    /// to the node from now on.
    async fn shell(&self, call: ToolInvocation, input: ShellInput) -> Result<ToolOutcome, CallError> {
        // A call after one that the user's send now returned never starts
        // (`runtime.md` § Send now).
        if call.arrival.sent_now() {
            return Ok(ToolOutcome::not_run());
        }
        let (interval_ms, note) = match input.interval_ms.0 {
            Some(asked) => {
                let (taken, note) = taken_interval(asked, self.interval_floor_ms, "intervalMs");
                (Some(taken), note)
            }
            None => (None, None),
        };
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
            interval_ms,
            call.arrival.arrived(),
        )
        .await?;
        let mut outcome = result::shell_outcome(
            &status,
            &call.model.model,
            call.request_limits,
            Look {
                note: note.as_deref(),
                sent_now: ended_by == Some(WindowEnd::SentNow),
                interval_ms: Some(interval_ms),
                report: false,
            },
        )
        .await;
        if matches!(status.state, CommandState::Running { .. }) {
            outcome.effect = Some(ToolEffect::Background {
                command,
                interval_ms,
                title: input.description.0,
            });
        } else {
            self.release(environment.as_ref(), &command).await;
        }
        Ok(outcome)
    }

    /// Forgets a command whose end a look showed the node: its handle is
    /// released, and `demi shell output` reads its output from then on.
    async fn release(&self, environment: &dyn ShellEnvironment, command: &CommandId) {
        self.environments.saw_end(command);
        // A command the environment already forgot has nothing to release.
        environment.release_command(command).await;
    }

    /// Writes `stdin` to a running command of the current Host, as a page's
    /// input does.
    pub async fn write(&self, command: &CommandId, stdin: String) -> Result<(), CallError> {
        let (_, environment) = self.environment(Handle::Command(command)).await?;
        environment.write(command, Bytes::from(stdin)).await?;
        Ok(())
    }

    /// Stops a running command of the current Host, as a page's stop does:
    /// its end's report says the user stopped it.
    pub async fn abort(&self, command: &CommandId) -> Result<(), CallError> {
        let (_, environment) = self.environment(Handle::Command(command)).await?;
        self.environments.stopped_by(command, Stopper::User);
        environment.abort(command).await?;
        Ok(())
    }
}

/// Watches `command` until the first of (`runtime.md` § The window): it
/// ends; `interval_ms` passes, or for a resident command, its output has
/// been quiet for 2 seconds or 30 seconds have passed; or `until` resolves.
/// Then returns its status, with what `until` gave when it ended the watch.
/// Ending the watch never stops the command.
async fn watch<T>(
    environment: &dyn ShellEnvironment,
    command: &CommandId,
    interval_ms: Option<u32>,
    until: impl Future<Output = T>,
) -> Result<(CommandStatus, Option<T>), ShellError> {
    let window = async {
        match interval_ms {
            Some(interval) => {
                tokio::time::sleep(Duration::from_millis(interval.into())).await;
                Ok(())
            }
            None => {
                tokio::select! {
                    quiet = quiet(environment, command) => quiet,
                    () = tokio::time::sleep(RESIDENT_WINDOW) => Ok(()),
                }
            }
        }
    };
    let mut ended_by = None;
    tokio::select! {
        ended = environment.ended(command) => {
            ended?;
        }
        watched = window => watched?,
        reason = until => ended_by = Some(reason),
    }
    Ok((environment.status(command)?, ended_by))
}

/// Resolves once `command`'s output has been quiet for [`QUIET`]; it waits
/// for the moment the quiet would be reached, and looks again then.
async fn quiet(environment: &dyn ShellEnvironment, command: &CommandId) -> Result<(), ShellError> {
    loop {
        let quiet = environment.quiet(command)?;
        if quiet >= QUIET {
            return Ok(());
        }
        tokio::time::sleep(QUIET - quiet).await;
    }
}

/// A call's outcome: a failure completes it as `Tool failed: <message>`.
fn outcome(ran: Result<ToolOutcome, CallError>) -> Result<ToolOutcome, ToolFailure> {
    ran.map_err(|CallError::Failed(message)| ToolFailure(message))
}

/// A duration as the model reads it: `45s`, `4m`, `1m5s`, `2h` or `1h3m`,
/// rounded to the nearest second.
pub fn duration(ms: u64) -> String {
    let seconds = (ms + 500) / 1000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return match seconds % 60 {
            0 => format!("{minutes}m"),
            rest => format!("{minutes}m{rest}s"),
        };
    }
    match minutes % 60 {
        0 => format!("{}h", minutes / 60),
        rest => format!("{}h{rest}m", minutes / 60),
    }
}

/// Why a call did not produce a result of its tool.
#[derive(Debug)]
pub enum CallError {
    /// The Host or its environment failed.
    Failed(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed(text) => formatter.write_str(text),
        }
    }
}

impl From<ShellError> for CallError {
    fn from(error: ShellError) -> Self {
        Self::Failed(error.to_string())
    }
}

