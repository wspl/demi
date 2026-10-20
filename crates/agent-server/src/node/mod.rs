//! A node of a conversation's tree and its assembly: every node, the root and
//! each subagent, is one kind of node built by one assembly, the one place that
//! creates a node's session. Parent and child are a relationship the tree
//! manages, never a second runtime, so the supervisor asks the assembly for a
//! node and never builds one itself. What differs between nodes is
//! configuration (instructions, model and provider runtime, the spawn
//! restriction, the role that sets the lifecycle policy); the role is never a
//! depth, and every child has its parent's commands.
//!
//! A child starts with an empty transcript: isolation is the point, so Demi
//! never rewrites or summarizes the parent's conversation into it. Its context
//! is the system prompt (a profile's instructions added to the product's, never
//! replacing them; the inherit profile copies its parent's whole), a preamble
//! naming its identity, and the brief as its first user message, which Demi
//! neither inspects nor pads. The context sources tell it what they tell the
//! parent. Its provider runtime is its own, closed with its session; a model
//! switch changes the root's selection only.
//!
//! A node's working directory is on the primary Host. The root's is the
//! conversation's; a child's starts as its parent's (or `--cwd`) and changes
//! only by its own `demi agent cwd`. When the conversation moves or its primary
//! Host changes, every child's directory becomes the conversation's new one,
//! since an absolute path of the old Host names nothing on the new one. A
//! child's instructions are read from its own directory; its skills are the
//! conversation's.

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::Arc,
};

use demi_agent_session::{
    AgentSession, CommandLook, Continuation, ForkSeed, NewContext, RestoreError, RunningWork, SeenContext,
    SessionConfig, SessionDeps, SessionInit, SessionRuntime, StepOutcomes, SystemPrompt,
    ToolInvocation,
};
use demi_agent_store::{
    AgentTreeStore, Checkpoint, ChildDirectory, NodeRecord, Placement, RunningCommand, StoreError,
    StoredCommand, StoredOutput,
    media::store_result,
};
use demi_agent_tools::{
    CallError, ContextSource, Directory, EndOf, Environments, HostResolver, Looking, ModelIdentity, NodeContext,
    ShellAccess, ShellEnvironmentFactory, Stopper, StoreNumbers, definitions, end_report,
    fill_output, interrupted_look, progress_report, report_media, runs_together, stored_running_commands,
    system_prompt, whole_status,
};
use demi_agent_transcript::IdSource;
use demi_host_interface::{
    CapabilityIndex, CommandSet, CommandState, CommandStatus, Ending, FileKind, Host, HostError, JobCaller, Numbers, PageFeed, PageState, PageView, Seen,
    ShellEnvironment,
};
use demi_provider_common::{ProviderRuntime, RequestLimits, ToolDefinition};
use demi_shared_gates::{ActivityGate, GateLease, Purpose, Reservation};
use demi_shared_types::{
    Clock, CommandEnd, CommandId, CommandReport, ModelSelection, ReportEvent, NodeId, QueuedMessage, TurnId,
    WORKING_DIRECTORY_SOURCE,
};
use futures_util::{FutureExt, future::LocalBoxFuture};

use crate::server::{AgentServer, ProviderResolver};

/// What made Demi resume a child's turn the process interrupted, as its
/// model reads it.
const RESTARTED: &str = "Demi resumed this turn after the backend restarted.";

/// A node's place in its tree, which sets its lifecycle policy: a child
/// resumes a turn the process interrupted and closes once it is quiescent;
/// the root leaves both to its client. A side question closes once it is
/// quiescent, and its interrupted turn is its card's to retry, as the
/// root's is its client's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeRole {
    Root,
    Child,
    Side,
}

/// One live node: its role, its session, and its runtime, which holds its
/// record.
pub struct Node<H: HostResolver> {
    role: NodeRole,
    session: AgentSession,
    runtime: Rc<NodeRuntime<H>>,
}

impl<H: HostResolver> Node<H> {
    pub fn id(&self) -> &NodeId {
        &self.runtime.node
    }

    /// The node's record as it is now: a child's working directory changes
    /// with `demi agent cwd`.
    pub fn record(&self) -> NodeRecord {
        self.runtime.record.borrow().clone()
    }

    /// The node that spawned it; none for the root.
    pub(crate) fn parent(&self) -> Option<NodeId> {
        self.runtime.record.borrow().parent.clone()
    }

    pub fn session(&self) -> &AgentSession {
        &self.session
    }

    /// Where the conversation works now.
    pub(crate) async fn placement(&self) -> Result<Placement, HostError> {
        self.runtime.placement().await
    }

    /// The node's working directory while the conversation works at `now`:
    /// a child's own while the conversation is where it was when that was
    /// set, the conversation's otherwise, and always the conversation's for
    /// the root.
    pub(crate) fn directory_at(&self, now: &Placement) -> String {
        self.runtime.directory_at(now)
    }

    /// `dir` read against the node's working directory while the
    /// conversation works at `now`, when that names a directory on the
    /// primary Host, which the host access reaches; why it does not
    /// otherwise.
    pub(crate) async fn existing_directory(&self, dir: &str, now: &Placement) -> Result<String, String> {
        let base = self.directory_at(now);
        let host = self
            .runtime
            .hosts
            .host(self.runtime.context_in(Some(&base)))
            .await
            .map_err(|error| error.to_string())?;
        let path = host.identity().paths.resolve(&base, dir);
        match host.fs().stat(&path).await {
            Ok(stat) if stat.kind == FileKind::Directory => Ok(path),
            Ok(_) => Err(format!("{path}: not a directory")),
            Err(error) if error.code() == Some("ENOENT") => {
                Err(format!("{path}: no such directory on the primary Host"))
            }
            Err(error) => Err(format!("{path}: {error}")),
        }
    }

    /// Moves the child to `cwd`, a directory on the primary Host, set while
    /// the conversation works at `under`, from its next `shell` call on:
    /// recorded, then the node's.
    pub(crate) async fn set_cwd(&self, cwd: String, under: Placement) -> Result<(), StoreError> {
        let own = ChildDirectory { cwd, under };
        self.runtime.store.set_cwd(&self.runtime.node, &own).await?;
        self.runtime.record.borrow_mut().cwd = Some(own);
        Ok(())
    }

    /// The commands the node's shell offers: the product's, with the
    /// `demi agent` group grafted. The backend's command router dispatches
    /// the node's `rpc` calls through them.
    pub fn commands(&self) -> &Rc<CommandSet> {
        &self.runtime.commands
    }

    /// The caller a job the node's call `tool_use_id` of `turn` starts
    /// records.
    pub fn job_caller(&self, tool_use_id: &str, turn: Option<TurnId>) -> JobCaller {
        JobCaller {
            node: self.runtime.node.clone(),
            tool_use_id: tool_use_id.to_owned(),
            turn,
        }
    }

    /// Writes `stdin` to a running command through the node's environment
    /// for the conversation's current Host, as a page's `shell_write` asks.
    pub(crate) async fn shell_write(
        &self,
        command: &CommandId,
        stdin: String,
    ) -> Result<(), CallError> {
        self.runtime.shell_access().write(command, stdin).await
    }

    /// Stops a running command through the node's environment for the
    /// conversation's current Host, as a page's `shell_abort` asks.
    pub(crate) async fn shell_abort(&self, command: &CommandId) -> Result<(), CallError> {
        self.runtime.shell_access().abort(command).await
    }

    /// `stopper` stops `command`, which one of the node's environments
    /// holds; its end's report says who (`crates/agent-session/src/reports.rs`).
    pub(crate) fn stopped_by(&self, command: &CommandId, stopper: Stopper) {
        self.runtime.environments.stopped_by(command, stopper);
    }

    /// The node's place in the whole output of `command`, which it looks at
    /// without holding it (`crates/agent-server/src/server/shell_output.rs`): as
    /// the node keeps it, or, for a running command it has not looked at
    /// since the backend started, as the command's record keeps it.
    pub(crate) async fn place(&self, command: &CommandId) -> Seen {
        if let Some(seen) = self.runtime.environments.known_place(command) {
            return seen;
        }
        self.runtime
            .store
            .place(command, &self.runtime.node)
            .await
            .unwrap_or_else(|error| {
                // The look shows the output from its start.
                tracing::warn!(node = %self.runtime.node, %command, %error, "a place in a command's output could not be read");
                None
            })
            .unwrap_or_default()
    }

    /// Moves the node's place in `command`'s whole output to `seen`, in the
    /// command's record too while it runs (`storage.md` § Command outputs).
    pub(crate) async fn set_place(&self, command: &CommandId, seen: Seen) {
        self.runtime.environments.set_place(command, seen);
        if let Err(error) = self.runtime.store.record_place(command, &self.runtime.node, seen).await {
            // The node keeps its place; a backend that starts again shows
            // it the output from its last recorded place.
            tracing::warn!(node = %self.runtime.node, %command, %error, "a place in a command's output was not recorded");
        }
    }

    /// A look showed the node `command`'s end, and all of its output, which
    /// one of its environments held: its handle is released, and its end
    /// tells the node nothing more.
    pub(crate) async fn saw_end(&self, command: &CommandId) {
        self.runtime.environments.saw_end(command);
        if let Some(environment) = self.runtime.environments.owning(command) {
            // A command the environment already forgot has nothing to
            // release.
            environment.release_command(command).await;
        }
    }

    /// The node looks at `commands` until the returned look is dropped: their
    /// end reports end no window meanwhile (`crates/agent-session/src/reports.rs`).
    pub(crate) fn look_at(&self, commands: Vec<CommandId>) -> Looking<'_> {
        self.runtime.environments.look_at(commands)
    }

    /// Whether a look of the node at `command` runs now.
    pub fn looks_at(&self, command: &CommandId) -> bool {
        self.runtime.environments.looks_at(command)
    }

    /// Whether one of the node's environments holds `command`.
    pub(crate) fn holds(&self, command: &CommandId) -> bool {
        self.runtime.environments.owning(command).is_some()
    }

    /// The node's environment that holds `command`.
    pub(crate) fn environment_of(&self, command: &CommandId) -> Option<Rc<dyn ShellEnvironment>> {
        self.runtime.environments.owning(command)
    }

    /// The pages' view of each live command of the node
    /// (`crates/agent-server/src/server/tree/live.rs`): each one its shells
    /// run, and each one the transcript last saw running that they still hold,
    /// oldest first.
    pub(crate) fn live_views(&self) -> Vec<PageView> {
        let stored = self.session.with_window(|transcript| stored_running_commands(transcript.blocks()));
        let mut views: Vec<PageView> = self
            .runtime
            .environments
            .page_views()
            .into_iter()
            .filter(|view| {
                matches!(view.state, PageState::Running | PageState::Waiting { .. } | PageState::Stopping)
                    || stored.contains(&view.command_id)
            })
            .collect();
        views.sort_by_key(|view| std::cmp::Reverse(view.running_ms));
        views
    }

    /// Takes up the node's command whose job is `job`, as its job's `rpc`
    /// call asks (`crates/agent-session/src/reports.rs`): none when the node ran
    /// no command the conversation records running with that job, else
    /// why it could not be taken up.
    pub(crate) async fn take_up_job(&self, job: &str) -> Option<Result<(), String>> {
        match self.runtime.take_up(|running| running.job == job).await {
            Ok(false) => None,
            Ok(true) => Some(Ok(())),
            Err(error) => Some(Err(error)),
        }
    }

    /// Ends each of the node's commands whose start still waits for its
    /// Host, as the user's Stop does.
    pub(crate) async fn stop_waiting(&self) {
        self.runtime.environments.stop_waiting().await;
    }

    /// Ends the node's shells on every Host, their running commands with
    /// them; a later tool call makes fresh ones.
    pub(crate) async fn end_shells(&self) {
        self.runtime.environments.end_all().await;
    }

    /// Takes up the commands that the closed node `closed` ran and the
    /// conversation records running, except those `held` says shells hold
    /// already, in shells made for it with this node's means and telling the
    /// pages through `feed`; asks each to stop again and hands the shells over
    /// (`crates/agent-server/src/server/tree/supervisor.rs`). A command whose
    /// Host cannot be reached now stays recorded running for the next restore.
    pub(crate) async fn take_up_closed(
        &self,
        closed: &NodeRecord,
        feed: Rc<dyn PageFeed>,
        held: impl Fn(&CommandId) -> bool,
    ) -> Vec<Rc<dyn ShellEnvironment>> {
        let runtime = &self.runtime;
        let running = match runtime.store.running_commands(&closed.id).await {
            Ok(running) => running,
            Err(error) => {
                tracing::warn!(node = %closed.id, %error, "a closed node's running commands could not be read");
                return Vec::new();
            }
        };
        let running: Vec<RunningCommand> = running.into_iter().filter(|command| !held(&command.command)).collect();
        if running.is_empty() {
            return Vec::new();
        }
        let environments = Environments::default();
        let access = ShellAccess {
            hosts: runtime.hosts.as_ref(),
            shells: runtime.shells.as_ref(),
            environments: &environments,
            node: &closed.id,
            root: &runtime.root,
            directory: &closed.cwd,
            agent: closed.number,
            commands: &runtime.commands,
            feed: &feed,
            numbers: &runtime.numbers,
            interval_floor_ms: runtime.interval_floor_ms,
            call_window_ms: runtime.call_window_ms,
        };
        // One left recorded running is tried again at the next restore.
        let _ = adopt_all(access, &running, runtime.clock.now().as_millisecond()).await;
        environments.stop_all().await
    }

    /// Asks the node's commands on every Host to stop and hands over its
    /// shells, which follow the commands until their runners report their
    /// ends; the node's disposal lets none of them go
    /// (`crates/agent-server/src/server/tree/supervisor.rs`).
    pub(crate) async fn stop_shells(&self) -> Vec<Rc<dyn ShellEnvironment>> {
        self.runtime.environments.stop_all().await
    }

    /// The product's commands a child of this node inherits: this node's,
    /// before the `demi agent` graft, which every node of the tree shares.
    pub(crate) fn inherited_commands(&self) -> &Rc<CommandSet> {
        &self.runtime.inherited
    }

    /// The instructions of the node's system prompt, which a child without
    /// a profile inherits whole.
    pub(crate) fn instructions(&self) -> &Instructions {
        &self.runtime.instructions
    }

    /// Held by each start and close of the node's children, and reserved by
    /// an edit of its transcript.
    pub(crate) fn lifecycle(&self) -> &ActivityGate {
        &self.runtime.lifecycle
    }

    /// Runs what a restored or new node has yet to run, under its role's
    /// policy (`crates/agent-server/src/server/tree/supervisor.rs`): a child resumes the turn the
    /// process interrupted, while the root, whose restore recorded the
    /// interruption, leaves the turn to its client; the queued messages run again in
    /// order; waiting input wakes the root, or is written into its last turn
    /// when that failed or was interrupted (`failures-and-recovery.md` § The
    /// unfinished turn), and wakes a child only once its supervisor has
    /// looked at it, which closes it instead when it is quiescent. What this
    /// changed is saved at once.
    pub(crate) async fn continue_from(&self, continuation: Continuation) -> Result<(), StoreError> {
        // The commands the node left running go on where their Host kept
        // them (`sessions-and-targets.md` § Recovery and persistence).
        // One its Host cannot be reached for stays recorded running; the
        // failure is logged, and a report or an rpc call of it tries again.
        let _ = self.runtime.take_up(|_| true).await;
        let session = &self.session;
        // The restore held a child, which its supervision releases at its
        // first look, and ended a root's interrupted turn with its record.
        if continuation.interrupted && self.role == NodeRole::Child {
            // The action reports its course as events, and a refusal means
            // the session is closing.
            drop(session.resume(Some(RESTARTED)));
        }
        for message in continuation.queued {
            // A restored session is live and has run nothing, so it admits
            // them; the actions report their course as events.
            drop(session.send_message(message));
        }
        session.wake();
        session.flush().await
    }
}

/// Takes up each of `running` in `access`'s shells, counting the time each
/// has run from its start to `now`, in milliseconds since the epoch; answers
/// why the last one that failed stays recorded running.
async fn adopt_all<H: HostResolver>(
    access: ShellAccess<'_, H>,
    running: &[RunningCommand],
    now: i64,
) -> Option<String> {
    let mut left = None;
    for command in running {
        // A clock that went back counts no time.
        let ran = u64::try_from(now - command.started.as_millisecond()).unwrap_or(0);
        if let Err(error) = access.adopt(command, std::time::Duration::from_millis(ran)).await {
            tracing::warn!(node = %access.node, command = %command.command, %error, "a running command was not taken up");
            left = Some(error.to_string());
        }
    }
    left
}

/// What the session calls in its node: the tree's admission, the system
/// prompt, the preamble, the context sources, the tools, the window its
/// model is used with, and the hold on its children that an edit needs.
pub(crate) struct NodeRuntime<H: HostResolver> {
    node: NodeId,
    root: NodeId,
    /// The server whose tree the node is in; weak, since the tree holds the
    /// node.
    server: Weak<AgentServer<H>>,
    /// Where the window in use for a model comes from.
    providers: Rc<dyn ProviderResolver>,
    /// The node's record, the one live copy: `demi agent cwd` changes a
    /// child's working directory in it.
    record: RefCell<NodeRecord>,
    hosts: Rc<H>,
    /// The instructions of its system prompt.
    instructions: Instructions,
    /// The product's harness guide.
    guide: Rc<str>,
    /// The capability index of its commands, rendered once.
    index: CapabilityIndex,
    /// A child's identity, the text before each of its user turns.
    preamble: Option<String>,
    /// The product's context sources, in their order.
    context: Rc<[Rc<dyn ContextSource>]>,
    inherited: Rc<CommandSet>,
    commands: Rc<CommandSet>,
    admission: ActivityGate,
    lifecycle: ActivityGate,
    store: Rc<dyn AgentTreeStore>,
    shells: Rc<dyn ShellEnvironmentFactory<H::Host>>,
    /// The node's shell environment on each Host its tools used.
    environments: Environments,
    /// Where its environments tell the pages of its commands.
    feed: Rc<dyn PageFeed>,
    /// The conversation's command numbers, from the tree store.
    numbers: Rc<dyn Numbers>,
    /// The node's agent number.
    agent: u64,
    /// The least interval its commands report at, in milliseconds.
    interval_floor_ms: u32,
    /// The longest a call of a command with an interval watches it, in
    /// milliseconds.
    call_window_ms: u32,
    /// Where the time a command taken up again has run comes from.
    clock: Arc<dyn Clock>,
}

/// The child's directory as its record holds it now.
impl<H: HostResolver> Directory for NodeRuntime<H> {
    fn own(&self) -> Option<ChildDirectory> {
        self.record.borrow().cwd.clone()
    }
}

/// What a child's `working_directory` block tells it.
fn working_directory_text(cwd: &str) -> String {
    format!(
        "Your working directory is {cwd}: your shell calls start there on the primary Host, and the relative paths you write are read against it. `demi agent cwd <dir>` changes it."
    )
}

/// The `working_directory` block a child's next request needs, given the
/// context blocks it receives, `seen`: one naming `cwd`, the child's
/// directory, unless the newest it receives names it already. A child has
/// one before its first request, after each change and after each
/// compaction; the root, whose directory is the conversation's that the
/// execution context names, none.
fn working_directory(cwd: Option<String>, seen: &[SeenContext<'_>]) -> Option<NewContext> {
    let cwd = cwd?;
    let text = working_directory_text(&cwd);
    let newest = seen
        .iter()
        .rev()
        .find(|seen| seen.source == WORKING_DIRECTORY_SOURCE)
        .map(|seen| seen.text);
    (newest != Some(text.as_str())).then(|| NewContext {
        source: WORKING_DIRECTORY_SOURCE.to_owned(),
        text,
        instructions: Vec::new(),
        cwd: Some(cwd),
    })
}

impl<H: HostResolver> NodeRuntime<H> {
    /// Where the conversation works now, read through its resolver.
    async fn placement(&self) -> Result<Placement, HostError> {
        self.hosts.placement(self.context_in(None)).await
    }

    /// The node's working directory while the conversation works at `now`.
    fn directory_at(&self, now: &Placement) -> String {
        match &self.record.borrow().cwd {
            Some(own) => own.effective(now).to_owned(),
            None => now.directory.clone(),
        }
    }

    /// A child's working directory now; none for the root, and none, with
    /// the failure logged, when where the conversation works cannot be read.
    async fn child_directory(&self) -> Option<String> {
        self.record.borrow().cwd.as_ref()?;
        match self.placement().await {
            Ok(now) => Some(self.directory_at(&now)),
            Err(error) => {
                tracing::warn!(node = %self.node, %error, "where the conversation works could not be read");
                None
            }
        }
    }

    /// Takes up the commands the node ran that the conversation records
    /// running, that `which` picks and its environments do not hold
    /// (`sessions-and-targets.md` § Recovery and persistence). Answers
    /// whether `which` picked one, or why one stays recorded running without
    /// being taken up, as when its Host cannot be reached now; the next
    /// restore tries again.
    async fn take_up(&self, which: impl Fn(&RunningCommand) -> bool) -> Result<bool, String> {
        let running = self.store.running_commands(&self.node).await.map_err(|error| {
            tracing::warn!(node = %self.node, %error, "the node's running commands could not be read");
            error.to_string()
        })?;
        let picked: Vec<RunningCommand> = running.into_iter().filter(|command| which(command)).collect();
        let unheld: Vec<RunningCommand> = picked
            .iter()
            .filter(|command| self.environments.owning(&command.command).is_none())
            .cloned()
            .collect();
        let left = adopt_all(self.shell_access(), &unheld, self.clock.now().as_millisecond()).await;
        left.map_or(Ok(!picked.is_empty()), Err)
    }

    /// The node as its context sources and resolver see it, with `cwd`,
    /// its own directory as the caller read it: a change made meanwhile
    /// shows at the next read.
    fn context_in<'a>(&'a self, cwd: Option<&'a str>) -> NodeContext<'a> {
        NodeContext {
            node: &self.node,
            root: &self.root,
            cwd,
        }
    }

    /// What the `shell` tool reaches the node's shells through.
    fn shell_access(&self) -> ShellAccess<'_, H> {
        ShellAccess {
            hosts: &self.hosts,
            shells: self.shells.as_ref(),
            environments: &self.environments,
            node: &self.node,
            root: &self.root,
            directory: self,
            agent: self.agent,
            commands: &self.commands,
            feed: &self.feed,
            numbers: &self.numbers,
            interval_floor_ms: self.interval_floor_ms,
            call_window_ms: self.call_window_ms,
        }
    }

    /// How `command`, which one of the node's environments held, ended: as
    /// the conversation's record of it keeps it (`storage.md` § Command
    /// outputs), which the record holds before the command reads as ended;
    /// a command no record keeps, as in a product without a keeper, ended
    /// as its environment says.
    async fn end_of(&self, command: &CommandId) -> EndOf {
        let recorded = self.store.command_end(command).await.unwrap_or_else(|error| {
            tracing::warn!(%command, %error, "how a command ended could not be read");
            None
        });
        let stopper = self.environments.stopper(command);
        let end = recorded.or_else(|| {
            let environment = self.environments.owning(command)?;
            match environment.ended(command).now_or_never()?.ok()? {
                Ending::Exited(exit_code) => Some(CommandEnd::Exited { exit_code }),
                Ending::Aborted => Some(CommandEnd::Stopped),
                Ending::Lost(reason) => Some(CommandEnd::Lost { reason }),
            }
        });
        match end {
            Some(CommandEnd::Exited { exit_code }) => EndOf::Exited(exit_code),
            Some(CommandEnd::Stopped) => EndOf::Stopped(stopper),
            Some(CommandEnd::Lost { reason }) => EndOf::Lost(reason),
            Some(CommandEnd::Unrecorded) | None => EndOf::Unrecorded,
        }
    }

    /// What the conversation stored of `command`, which ended and no
    /// environment holds any more; none when it stored nothing of it.
    async fn stored(&self, command: &CommandId) -> Option<StoredCommand> {
        self.store.command_output(command).await.unwrap_or_else(|error| {
            tracing::warn!(%command, %error, "the output of an ended command could not be read");
            None
        })
    }

    /// What `command`, which ended and no environment holds any more, shows
    /// of its output since the node's last look, from `stored`, the output
    /// the conversation stored; reading it moves no place. None when the
    /// conversation keeps no output of it.
    fn stored_status(&self, command: &CommandId, stored: StoredCommand) -> Option<CommandStatus> {
        let StoredOutput::Stored { output, .. } = stored.output else {
            return None;
        };
        let state = match stored.end {
            CommandEnd::Exited { exit_code } => CommandState::Exited {
                exit_code,
                binary_stdout: None,
                media: Vec::new(),
            },
            CommandEnd::Lost { reason } => CommandState::Lost {
                reason,
                media: Vec::new(),
            },
            // A record without its end shows its output as a stopped
            // command's.
            CommandEnd::Stopped | CommandEnd::Unrecorded => CommandState::Aborted,
        };
        let place = self.environments.place(command);
        Some(whole_status(command, state, 0, 0, Arc::new(output), place))
    }
}

impl<H: HostResolver> SessionRuntime for NodeRuntime<H> {
    fn enter_action(&self) -> LocalBoxFuture<'_, GateLease> {
        Box::pin(self.admission.enter(Purpose::Demand))
    }

    /// The root's is where the conversation works, a child's its own while
    /// the conversation works where it was set.
    fn directory(&self) -> LocalBoxFuture<'_, Option<String>> {
        Box::pin(async move {
            match self.placement().await {
                Ok(now) => Some(self.directory_at(&now)),
                Err(error) => {
                    tracing::warn!(node = %self.node, %error, "where the conversation works could not be read");
                    None
                }
            }
        })
    }

    /// A root's turns and a side question's, whose card offers Retry: a
    /// child's result says what it could not do, and its parent's turn
    /// decides (`failures-and-recovery.md` § The unfinished turn).
    fn user_recovers_turns(&self) -> bool {
        self.node == self.root || self.record.borrow().side.is_some()
    }

    fn reserve_edit(&self) -> LocalBoxFuture<'_, Result<Option<Reservation>, String>> {
        Box::pin(async move {
            let reservation = self.lifecycle.try_reserve().ok_or_else(|| {
                "Cannot edit while a child lifecycle operation is in progress".to_owned()
            })?;
            let children = self
                .store
                .children(&self.node)
                .await
                .map_err(|error| error.to_string())?;
            if children
                .iter()
                .any(|child| child.closed.is_none() || !child.delivered)
            {
                return Err(
                    "Cannot edit while children or completion notifications are pending".to_owned(),
                );
            }
            Ok(Some(reservation))
        })
    }

    /// The layers rendered at assembly, and the line naming `model` with
    /// the family of the entry that serves it.
    fn system_prompt<'a>(
        &'a self,
        model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<SystemPrompt, String>> {
        Box::pin(async move {
            let family = self
                .providers
                .family(model)
                .await
                .map_err(|error| error.to_string())?;
            let identity = ModelIdentity {
                name: &model.model.name,
                family: &family,
                id: &model.model.id,
            };
            Ok(system_prompt(
                &self.instructions.identity,
                &self.guide,
                &self.index,
                identity,
                self.instructions.added.as_deref(),
            ))
        })
    }

    /// The window the product gives `model`, with the Context the node's
    /// profile chose in place of the user's limit.
    fn context_window<'a>(&'a self, model: &'a ModelSelection) -> LocalBoxFuture<'a, u32> {
        let own = self.record.borrow().context_limit;
        self.providers.context_window(model, own)
    }

    fn preamble(&self) -> LocalBoxFuture<'_, Option<String>> {
        let text = self.preamble.clone();
        Box::pin(async move { text })
    }

    /// Asks every source at once, each with its own blocks, and takes
    /// their answers in the sources' order: a source that reads a Host does
    /// not make the others wait. A source that fails adds nothing: it is
    /// asked again before the next request.
    fn context<'a>(
        &'a self,
        seen: &'a [SeenContext<'a>],
        turn: &'a TurnId,
    ) -> LocalBoxFuture<'a, Vec<NewContext>> {
        Box::pin(async move {
            let cwd = self.child_directory().await;
            let node = self.context_in(cwd.as_deref());
            let asked = self.context.iter().map(|source| async move {
                let name = source.name();
                let own: Vec<&str> = seen
                    .iter()
                    .filter(|seen| seen.source == name)
                    .map(|seen| seen.text)
                    .collect();
                (name, source.context(node, turn, &own).await)
            });
            let mut news = Vec::new();
            for (name, answer) in futures_util::future::join_all(asked).await {
                match answer {
                    Ok(Some(answer)) => news.push(NewContext {
                        source: name.to_owned(),
                        text: answer.text,
                        instructions: answer.instructions,
                        cwd: None,
                    }),
                    Ok(None) => {}
                    Err(error) => tracing::warn!(
                        node = %self.node,
                        source = name,
                        %error,
                        "a context source failed; it is asked again before the next request"
                    ),
                }
            }
            news.extend(working_directory(cwd, seen));
            news
        })
    }

    /// The `shell` tool, and only it.
    fn tools(&self) -> Arc<[ToolDefinition]> {
        definitions()
    }

    /// None for a side question, which is no agent.
    fn agent(&self) -> Option<u64> {
        self.record.borrow().side.is_none().then_some(self.agent)
    }

    /// A side question's requests name the root's session to their
    /// provider, so they read the conversation's prompt cache.
    fn affinity(&self) -> Option<String> {
        self.record.borrow().side.is_some().then(|| self.root.to_string())
    }

    /// The commands the node's shells run, oldest first, each with its
    /// call's title, and the node's live children in spawn order.
    fn running_work(&self) -> LocalBoxFuture<'_, RunningWork> {
        let mut views: Vec<PageView> = self
            .environments
            .page_views()
            .into_iter()
            .filter(|view| matches!(view.state, PageState::Running | PageState::Waiting { .. }))
            .collect();
        views.sort_by_key(|view| std::cmp::Reverse(view.running_ms));
        let commands = views
            .into_iter()
            .map(|view| {
                let title = self
                    .environments
                    .owning(&view.command_id)
                    .and_then(|environment| environment.title(&view.command_id))
                    .unwrap_or_default();
                (view.command_id, title)
            })
            .collect();
        let subagents = self
            .server
            .upgrade()
            .and_then(|server| server.tree(&self.root))
            .map(|tree| tree.live_children_of(&self.node))
            .unwrap_or_default();
        Box::pin(async move {
            RunningWork {
                commands,
                subagents,
            }
        })
    }

    fn runs_together(&self, tool: &str) -> bool {
        runs_together(tool)
    }

    fn invoke_step(&self, calls: Vec<ToolInvocation>) -> StepOutcomes<'_> {
        self.shell_access().invoke_step(calls)
    }

    /// Waits on the environment that holds `command`; one no environment
    /// holds is taken up when the conversation records it running, and has
    /// ended otherwise, with the node's shells or before the node was
    /// restored.
    fn awaits_user(&self, command: &CommandId) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.environments.owning(command)?.awaits_user(command)
    }

    fn command_ended<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            // A command its Host cannot be reached for now has not ended:
            // it reports nothing until the node takes it up.
            if self.environments.owning(command).is_none()
                && self.take_up(|running| running.command == *command).await.is_err()
            {
                std::future::pending::<()>().await;
            }
            if let Some(environment) = self.environments.owning(command) {
                // An environment that forgot the command answers an error:
                // it has ended either way.
                let _ = environment.ended(command).await;
            }
        })
    }

    /// A running command's progress, or its end, unless a look showed it
    /// the node already or the node stopped it itself. The output of a
    /// command an environment holds is read when the report is written
    /// ([`SessionRuntime::write_report`]); a command none holds any more is
    /// read from its stored output now, which moves no place.
    fn report<'a>(
        &'a self,
        command: &'a CommandId,
        interval_ms: Option<u32>,
        model: &'a ModelSelection,
        limits: RequestLimits,
    ) -> LocalBoxFuture<'a, Option<CommandReport>> {
        Box::pin(async move {
            // The media the job viewed, which only its end report attaches:
            // a look that showed the end attached none.
            let media = self
                .environments
                .owning(command)
                .and_then(|environment| environment.media(command).ok())
                .unwrap_or_default();
            if self.environments.end_seen(command) && media.is_empty() {
                return None;
            }
            if let Some(environment) = self.environments.owning(command)
                && environment.ended(command).now_or_never().is_none()
            {
                // A resident command reports only its end, and one whose Host's
                // connection is away nothing until it is back: the silence is
                // the connection's (`crates/agent-session/src/reports.rs`). So
                // does one whose start waits for its Host, which never ran
                // (`sessions-and-targets.md` § Host operations).
                let interval = interval_ms?;
                if environment.unreachable(command).is_some() {
                    return None;
                }
                let view = environment
                    .page_views()
                    .into_iter()
                    .find(|view| &view.command_id == command);
                if view.as_ref().is_some_and(|view| matches!(view.state, PageState::Waiting { .. })) {
                    return None;
                }
                let running_ms = view.map_or(0, |view| view.running_ms);
                let idle_ms = environment
                    .quiet(command)
                    .map_or(0, |quiet| u64::try_from(quiet.as_millis()).unwrap_or(u64::MAX));
                let title = environment.title(command).unwrap_or_default();
                return Some(progress_report(command, &title, running_ms, idle_ms, interval));
            }
            if self.environments.stopper(command) == Some(Stopper::Itself) {
                return None;
            }
            let end = self.end_of(command).await;
            // Its record names it by the title of the call that started it,
            // whatever became of that call (`crates/agent-session/src/reports.rs`).
            let (title, status) = match self.environments.owning(command) {
                Some(environment) => (environment.title(command).unwrap_or_default(), None),
                None => match self.stored(command).await {
                    Some(stored) => (stored.title.clone(), self.stored_status(command, stored)),
                    None => (String::new(), None),
                },
            };
            let mut report = end_report(command, &title, end, status.as_ref());
            if !media.is_empty() {
                let parts = report_media(&media, &model.model, limits).await;
                let session = self.store.session_store(&self.node);
                // The held bytes are read again before a request needs them
                // (`transcript.md` § Media).
                let (stored, _held) = store_result(parts, session.blobs()).await;
                report.media = stored;
            }
            Some(report)
        })
    }

    fn end_seen(&self, command: &CommandId) -> bool {
        self.environments.end_seen(command)
    }

    /// Looks at a command an environment holds as a result shows it,
    /// which moves the node's place in its output.
    fn look(&self, command: &CommandId, budget: usize) -> Option<CommandLook> {
        let status = self.environments.owning(command)?.status(command).ok()?;
        let (output, view) = interrupted_look(&status, budget);
        Some(CommandLook { output, view })
    }

    fn looks_at(&self, command: &CommandId) -> bool {
        self.environments.looks_at(command)
    }

    /// Reads the output of a command an environment holds now, which moves
    /// the node's place in it; an end report's end is the model's from then
    /// on, as a look's is.
    fn write_report(&self, report: &mut CommandReport) {
        let command = report.command_id.clone();
        if let Some(status) = self
            .environments
            .owning(&command)
            .and_then(|environment| environment.status(&command).ok())
        {
            fill_output(report, &status);
        }
        if !matches!(report.event, ReportEvent::Running { .. }) {
            self.environments.saw_end(&command);
        }
    }

    /// Lets go of the node's shells on every Host; their running commands
    /// run on (`runtime.md` § Dispose and restore).
    fn dispose(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(self.environments.dispose())
    }
}

/// Where a node comes from: created now, empty or from a Fork's seed, or
/// restored from what the tree store holds of it.
pub(crate) enum Origin {
    New {
        record: NodeRecord,
        model: ModelSelection,
        /// Its first message, queued in its create commit, so that a node
        /// the process loses before its first save still has it.
        first_message: Option<QueuedMessage>,
    },
    /// A forked child (`crates/agent-server/src/server/tree/supervisor.rs`):
    /// its first checkpoint is the seed of its source's history, with its first
    /// message queued, created in one commit and then restored as a stored node
    /// is.
    Forked {
        record: NodeRecord,
        seed: ForkSeed,
    },
    Stored {
        record: NodeRecord,
        checkpoint: Checkpoint,
    },
}

impl Origin {
    /// What the tree store holds of the node `id`; none when it holds no
    /// such node.
    pub(crate) async fn stored(
        store: &dyn AgentTreeStore,
        id: &NodeId,
    ) -> Result<Option<Self>, StoreError> {
        let Some(record) = store.node(id).await? else {
            return Ok(None);
        };
        let checkpoint = store
            .session_store(id)
            .load()
            .await?
            .ok_or_else(|| StoreError::Corrupt(format!("node {id} has no checkpoint")))?;
        Ok(Some(Self::Stored { record, checkpoint }))
    }

    pub(crate) fn record(&self) -> &NodeRecord {
        match self {
            Self::New { record, .. } | Self::Forked { record, .. } | Self::Stored { record, .. } => {
                record
            }
        }
    }

    /// The model selection the node infers with: a new node's, or the one
    /// its checkpoint saved.
    pub(crate) fn model(&self) -> &ModelSelection {
        match self {
            Self::New { model, .. } => model,
            Self::Forked { seed, .. } => &seed.checkpoint.state.model,
            Self::Stored { checkpoint, .. } => &checkpoint.state.model,
        }
    }
}

/// The instructions of a node's system prompt (`system-prompt.md`
/// § Identity): the product's, its identity, which every node has, and the
/// text its subagent profile added after the model identity, if any.
#[derive(Debug, Clone)]
pub(crate) struct Instructions {
    pub(crate) identity: Rc<str>,
    pub(crate) added: Option<Rc<str>>,
}

/// What makes a node itself, and what it is built with.
pub(crate) struct NodeSpec<H: HostResolver> {
    pub(crate) origin: Origin,
    pub(crate) role: NodeRole,
    pub(crate) root: NodeId,
    /// The server whose tree the node is in, which knows its live children.
    pub(crate) server: Weak<AgentServer<H>>,
    /// A runtime that serves the origin's model selection's provider entry.
    pub(crate) runtime: Box<dyn ProviderRuntime>,
    pub(crate) providers: Rc<dyn ProviderResolver>,
    pub(crate) hosts: Rc<H>,
    pub(crate) instructions: Instructions,
    /// The product's harness guide, which every node carries.
    pub(crate) guide: Rc<str>,
    /// A child's identity, the text before each of its user turns.
    pub(crate) preamble: Option<String>,
    pub(crate) context: Rc<[Rc<dyn ContextSource>]>,
    /// The product's commands before the `demi agent` graft: what the
    /// node's own children inherit.
    pub(crate) inherited: Rc<CommandSet>,
    /// `inherited` with the node's `demi agent` group grafted.
    pub(crate) commands: Rc<CommandSet>,
    pub(crate) store: Rc<dyn AgentTreeStore>,
    pub(crate) shells: Rc<dyn ShellEnvironmentFactory<H::Host>>,
    /// Where the node's environments tell the pages of its commands: the
    /// node's feed of its tree.
    pub(crate) feed: Rc<dyn PageFeed>,
    pub(crate) admission: ActivityGate,
    pub(crate) ids: Rc<dyn IdSource>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) config: SessionConfig,
    /// The least interval its commands report at, in milliseconds.
    pub(crate) interval_floor_ms: u32,
    /// The longest a call of a command with an interval watches it, in
    /// milliseconds.
    pub(crate) call_window_ms: u32,
}

/// A node, and what it has yet to run: what its restore handed back, or a new
/// node's first message.
pub(crate) struct Assembled<H: HostResolver> {
    pub(crate) node: Node<H>,
    pub(crate) continuation: Option<Continuation>,
}

/// Why a node could not be assembled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum AssembleError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Restore(#[from] RestoreError),
}

/// The one way a node comes to exist: restored from what the tree store
/// holds of it, or created, its record and first checkpoint in one commit.
/// When assembly fails, the provider runtime it was given served no run,
/// and dropping it releases what it holds, the provider contract's fallback
/// for a runtime that is not closed.
pub(crate) async fn assemble<H: HostResolver>(
    spec: NodeSpec<H>,
) -> Result<Assembled<H>, AssembleError> {
    let NodeSpec {
        origin,
        role,
        root,
        server,
        runtime,
        providers,
        hosts,
        instructions,
        guide,
        preamble,
        context,
        inherited,
        commands,
        store,
        shells,
        feed,
        admission,
        ids,
        clock,
        config,
        interval_floor_ms,
        call_window_ms,
    } = spec;
    let record = origin.record().clone();
    let session_store = store.session_store(&record.id);
    let index = commands.render_index();
    let node_runtime = Rc::new(NodeRuntime {
        node: record.id.clone(),
        root,
        server,
        providers,
        record: RefCell::new(record.clone()),
        hosts,
        instructions,
        guide,
        index,
        preamble,
        context,
        inherited,
        commands,
        admission,
        lifecycle: ActivityGate::new(),
        numbers: Rc::new(StoreNumbers(store.clone())),
        agent: record.number,
        interval_floor_ms,
        call_window_ms,
        store: store.clone(),
        shells,
        environments: Environments::default(),
        feed,
        clock: clock.clone(),
    });
    let deps = SessionDeps {
        runtime: node_runtime.clone(),
        store: session_store,
        ids,
        clock,
        config,
    };
    let (session, continuation) = match origin {
        Origin::Stored { mut checkpoint, .. } => {
            let running = store.running_commands(&record.id).await?;
            // A report that waited in the checkpoint for a command whose
            // shells were let go carries what the command's stored output
            // holds since the node's place, as a report of a command no
            // shells hold does (`crates/agent-session/src/reports.rs`).
            let state = &mut checkpoint.state;
            for report in state.reports.iter_mut().chain(&mut state.next_turn_reports) {
                let ended = !matches!(report.event, ReportEvent::Running { .. });
                let recorded = running.iter().any(|running| running.command == report.command_id);
                if ended && report.output.is_empty() && !recorded
                    && let Some(stored) = node_runtime.stored(&report.command_id).await
                    && let Some(status) = node_runtime.stored_status(&report.command_id, stored)
                {
                    fill_output(report, &status);
                }
            }
            let (session, continuation) =
                AgentSession::restore(checkpoint, record.id.clone(), runtime, deps, &running).await?;
            (session, Some(continuation))
        }
        Origin::Forked { seed, .. } => {
            // Its rows before the seed's blocks are its tree's own.
            seed.create(&*store, record.clone(), None).await?;
            // It restores as a stored node does: the copied rows are the
            // store's, and its window starts at its last boundary.
            let checkpoint = match seed.prefix {
                None => seed.checkpoint,
                Some(_) => store
                    .session_store(&record.id)
                    .load()
                    .await?
                    .ok_or_else(|| StoreError::Failed(format!("node {} has no checkpoint", record.id)))?,
            };
            // A new node has no commands of its own yet.
            let (session, continuation) =
                AgentSession::restore(checkpoint, record.id.clone(), runtime, deps, &[]).await?;
            (session, Some(continuation))
        }
        Origin::New {
            model,
            first_message,
            ..
        } => {
            let init = SessionInit {
                id: record.id.clone(),
                model,
                runtime,
            };
            let session = AgentSession::create(init, deps);
            let mut initial = session.first_checkpoint();
            initial.state.queue.extend(first_message.clone());
            store.create_node(record.clone(), initial).await?;
            let continuation = first_message.map(|message| Continuation {
                interrupted: false,
                queued: vec![message],
            });
            (session, continuation)
        }
    };
    Ok(Assembled {
        node: Node {
            role,
            session,
            runtime: node_runtime,
        },
        continuation,
    })
}
