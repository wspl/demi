//! A node of a conversation's tree and its assembly (`subagents.md`
//! § Runtime): every node, the root and each subagent, is one kind of node
//! built by one assembly, which is the one place that creates a node's
//! session. What differs between nodes is configuration: the instructions,
//! the model, the commands, the spawn restriction, and the role that sets
//! the lifecycle policy.

use std::{
    rc::Rc,
    sync::Arc,
};



use demi_agent_session::{
    AgentSession, Continuation, NewContext, RestoreError, SeenContext, SessionConfig, SessionDeps,
    SessionInit, SessionRuntime, StepOutcomes, ToolInvocation,
};
use demi_agent_store::{
    AgentTreeStore, Checkpoint, NodeRecord, StoreError, StoredOutput, media::store_result,
};
use demi_agent_tools::{
    CallError, ContextSource, EndOf, Environments, HostResolver, Looking, ModelIdentity, NodeContext,
    ShellAccess, ShellEnvironmentFactory, Stopper, StoreNumbers, definitions, end_report,
    fill_output, progress_report, report_media, runs_together, stored_running_commands,
    system_prompt, whole_status,
};
use demi_agent_transcript::IdSource;
use demi_host_interface::{
    CommandSet, CommandState, CommandStatus, Ending, JobCaller, Numbers, PageFeed, PageState, PageView, Seen,
    ShellEnvironment, ShellError, WholeOutput,
};
use demi_provider_common::{ProviderRuntime, RequestLimits, ToolDefinition};
use demi_shared_gates::{ActivityGate, GateLease, Purpose, Reservation};
use demi_shared_types::{
    Clock, CommandEnd, CommandId, CommandReport, ModelSelection, ReportEvent, NodeId, QueuedMessage, TurnId,
};
use futures_util::{FutureExt, future::LocalBoxFuture};

use crate::server::ProviderResolver;

/// What made Demi resume a child's turn the process interrupted, as its
/// model reads it.
const RESTARTED: &str = "the backend restarted";

/// A node's place in its tree, which sets its lifecycle policy: a child
/// resumes a turn the process interrupted and closes once it is quiescent;
/// the root leaves both to its client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeRole {
    Root,
    Child,
}

/// One live node: its record, its role and its session.
pub struct Node<H: HostResolver> {
    record: NodeRecord,
    role: NodeRole,
    session: AgentSession,
    runtime: Rc<NodeRuntime<H>>,
}

impl<H: HostResolver> Node<H> {
    pub fn id(&self) -> &NodeId {
        &self.record.id
    }

    pub fn record(&self) -> &NodeRecord {
        &self.record
    }

    pub fn session(&self) -> &AgentSession {
        &self.session
    }

    /// The node's working directory.
    pub fn cwd(&self) -> &str {
        &self.runtime.cwd
    }

    /// The commands the node's shell offers: the product's, with the
    /// `demi agent` group grafted. The backend's command router dispatches
    /// the node's `rpc` calls through them.
    pub fn commands(&self) -> &Rc<CommandSet> {
        &self.runtime.commands
    }

    /// The caller a job the node starts records: this node.
    pub fn job_caller(&self) -> JobCaller {
        JobCaller {
            node: self.record.id.clone(),
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
    /// holds; its end's report says who (`runtime.md` § Command reports).
    pub(crate) fn stopped_by(&self, command: &CommandId, stopper: Stopper) {
        self.runtime.environments.stopped_by(command, stopper);
    }

    /// The node's place in the whole output of `command`, which it looks at
    /// without holding it (`runtime.md` § The `demi shell` commands).
    pub(crate) fn place(&self, command: &CommandId) -> Seen {
        self.runtime.environments.place(command)
    }

    /// Moves the node's place in `command`'s whole output to `seen`.
    pub(crate) fn set_place(&self, command: &CommandId, seen: Seen) {
        self.runtime.environments.set_place(command, seen);
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
    /// end reports end no window meanwhile (`runtime.md` § Command reports).
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

    /// What the Host of `command`, which one of the node's environments
    /// runs, has kept of its output so far.
    pub(crate) async fn read_output(&self, command: &CommandId) -> Result<WholeOutput, ShellError> {
        let environment = self
            .runtime
            .environments
            .owning(command)
            .ok_or_else(|| ShellError::UnknownCommand(command.clone()))?;
        environment.read_output(command).await
    }

    /// The pages' view of each live command of the node (`runtime.md`
    /// § Live output): each one its shells run, and each one the transcript
    /// last saw running that they still hold, oldest first.
    pub(crate) fn live_views(&self) -> Vec<PageView> {
        let stored = stored_running_commands(&self.session.transcript().blocks);
        let mut views: Vec<PageView> = self
            .runtime
            .environments
            .page_views()
            .into_iter()
            .filter(|view| view.state == PageState::Running || stored.contains(&view.command_id))
            .collect();
        views.sort_by_key(|view| std::cmp::Reverse(view.running_ms));
        views
    }

    /// Ends the node's shells on every Host, their running commands with
    /// them; a later tool call makes fresh ones.
    pub(crate) async fn end_shells(&self) {
        self.runtime.environments.end_all().await;
    }

    /// The product's commands a child of this node inherits: this node's,
    /// before the `demi agent` graft, which every node of the tree shares.
    pub(crate) fn inherited_commands(&self) -> &Rc<CommandSet> {
        &self.runtime.inherited
    }

    /// The instructions of the node's system prompt, which a child without
    /// a profile's inherits.
    pub(crate) fn instructions(&self) -> &Rc<str> {
        &self.runtime.instructions
    }

    /// Held by each start and close of the node's children, and reserved by
    /// an edit of its transcript.
    pub(crate) fn lifecycle(&self) -> &ActivityGate {
        &self.runtime.lifecycle
    }

    /// Runs what a restored or new node has yet to run, under its role's
    /// policy (`subagents.md` § Persistence): a child resumes the turn the
    /// process interrupted, while the root records the interruption and
    /// leaves the turn to its client; the queued messages run again in
    /// order; waiting input wakes the root only when its last turn was not
    /// interrupted, and a child only once its supervisor has
    /// looked at it, which closes it instead when it is quiescent. What this
    /// changed is saved at once.
    pub(crate) async fn continue_from(&self, continuation: Continuation) -> Result<(), StoreError> {
        // The commands the node left running go on where their Host kept
        // them (`sessions-and-targets.md` § Recovery and persistence).
        // One its Host cannot be reached for stays recorded running.
        self.runtime.take_up(None).await;
        let session = &self.session;
        if self.role == NodeRole::Child {
            // Its supervision releases the hold at its first look; an action
            // that starts first releases it too.
            session.hold();
        }
        if continuation.interrupted {
            match self.role {
                NodeRole::Root => session.record_interruption(),
                // The action reports its course as events, and a refusal
                // means the session is closing.
                NodeRole::Child => drop(session.resume_after(RESTARTED)),
            }
        }
        for message in continuation.queued {
            // A restored session is live and has run nothing, so it admits
            // them; the actions report their course as events.
            drop(session.send(message.content, message.id));
        }
        if !continuation.interrupted {
            session.wake();
        }
        session.flush().await
    }
}

/// What the session calls in its node: the tree's admission, the system
/// prompt, the preamble, the context sources, the tools, the window its
/// model is used with, and the hold on its children that an edit needs.
pub(crate) struct NodeRuntime<H: HostResolver> {
    node: NodeId,
    root: NodeId,
    /// Where the window in use for a model comes from.
    providers: Rc<dyn ProviderResolver>,
    cwd: String,
    hosts: Rc<H>,
    /// The instructions of its system prompt, its identity.
    instructions: Rc<str>,
    /// The product's harness guide.
    guide: Rc<str>,
    /// The capability index of its commands, rendered once.
    index: String,
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
}

impl<H: HostResolver> NodeRuntime<H> {
    /// Takes up the commands the node ran that the conversation records
    /// running and its environments do not hold: `only`, or every one
    /// (`sessions-and-targets.md` § Recovery and persistence). Answers
    /// whether one stays recorded running without being taken up, as when
    /// its Host cannot be reached now; the next restore tries again.
    async fn take_up(&self, only: Option<&CommandId>) -> bool {
        let running = match self.store.running_commands(&self.node).await {
            Ok(running) => running,
            Err(error) => {
                tracing::warn!(node = %self.node, %error, "the node's running commands could not be read");
                return only.is_some();
            }
        };
        let mut left = false;
        for command in running {
            if only.is_some_and(|only| *only != command.command)
                || self.environments.owning(&command.command).is_some()
            {
                continue;
            }
            if let Err(error) = self.shell_access().adopt(&command).await {
                tracing::warn!(node = %self.node, command = %command.command, %error, "a running command was not taken up");
                left = true;
            }
        }
        left
    }

    fn node_context(&self) -> NodeContext<'_> {
        NodeContext {
            node: &self.node,
            root: &self.root,
            cwd: &self.cwd,
        }
    }

    /// What the `shell` tool reaches the node's shells through.
    fn shell_access(&self) -> ShellAccess<'_, H> {
        ShellAccess {
            hosts: &self.hosts,
            shells: self.shells.as_ref(),
            environments: &self.environments,
            context: self.node_context(),
            agent: self.agent,
            commands: &self.commands,
            feed: &self.feed,
            numbers: &self.numbers,
            interval_floor_ms: self.interval_floor_ms,
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
            }
        });
        match end {
            Some(CommandEnd::Exited { exit_code }) => EndOf::Exited(exit_code),
            Some(CommandEnd::Stopped) => EndOf::Stopped(stopper),
            Some(CommandEnd::Lost { reason }) => EndOf::Lost(reason),
            Some(CommandEnd::Unrecorded) | None => EndOf::Unrecorded,
        }
    }

    /// What `command`, which ended and no environment holds any more, shows
    /// of its output since the node's last look, from the output the
    /// conversation stored; reading it moves no place. None when the
    /// conversation keeps no output of it.
    async fn stored_status(&self, command: &CommandId) -> Option<CommandStatus> {
        let stored = self.store.command_output(command).await.unwrap_or_else(|error| {
            tracing::warn!(%command, %error, "the output of an ended command could not be read");
            None
        })?;
        let StoredOutput::Stored { output, .. } = stored.output else {
            return None;
        };
        let state = match stored.end {
            CommandEnd::Exited { exit_code } => CommandState::Exited {
                exit_code,
                binary_stdout: None,
                media: Vec::new(),
            },
            CommandEnd::Stopped | CommandEnd::Lost { .. } | CommandEnd::Unrecorded => {
                CommandState::Aborted
            }
        };
        let place = self.environments.place(command);
        Some(whole_status(command, state, 0, 0, Arc::new(output), place))
    }
}

impl<H: HostResolver> SessionRuntime for NodeRuntime<H> {
    fn enter_action(&self) -> LocalBoxFuture<'_, GateLease> {
        Box::pin(self.admission.enter(Purpose::Demand))
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
    ) -> LocalBoxFuture<'a, Result<String, String>> {
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
                &self.instructions,
                &self.guide,
                &self.index,
                identity,
            ))
        })
    }

    fn context_window<'a>(&'a self, model: &'a ModelSelection) -> LocalBoxFuture<'a, u32> {
        self.providers.context_window(model)
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
            let asked = self.context.iter().map(|source| async move {
                let name = source.name();
                let own: Vec<&str> = seen
                    .iter()
                    .filter(|seen| seen.source == name)
                    .map(|seen| seen.text)
                    .collect();
                (name, source.context(self.node_context(), turn, &own).await)
            });
            let mut news = Vec::new();
            for (name, answer) in futures_util::future::join_all(asked).await {
                match answer {
                    Ok(Some(answer)) => news.push(NewContext {
                        source: name.to_owned(),
                        text: answer.text,
                        instructions: answer.instructions,
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
            news
        })
    }

    /// The `shell` tool, and only it.
    fn tools(&self) -> Arc<[ToolDefinition]> {
        definitions()
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
    fn command_ended<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            // A command its Host cannot be reached for now has not ended:
            // it reports nothing until the node takes it up.
            if self.environments.owning(command).is_none() && self.take_up(Some(command)).await {
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
        title: &'a str,
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
                // A resident command reports only its end.
                let interval = interval_ms?;
                let running_ms = environment
                    .page_views()
                    .into_iter()
                    .find(|view| &view.command_id == command)
                    .map_or(0, |view| view.running_ms);
                let idle_ms = environment
                    .quiet(command)
                    .map_or(0, |quiet| u64::try_from(quiet.as_millis()).unwrap_or(u64::MAX));
                return Some(progress_report(command, title, running_ms, idle_ms, interval));
            }
            if self.environments.stopper(command) == Some(Stopper::Itself) {
                return None;
            }
            let end = self.end_of(command).await;
            let status = match self.environments.owning(command) {
                Some(_) => None,
                None => self.stored_status(command).await,
            };
            let mut report = end_report(command, title, end, status.as_ref());
            if !media.is_empty() {
                let parts = report_media(&media, &model.model, limits).await;
                let session = self.store.session_store(&self.node);
                // The held bytes are read again before a request needs them
                // (`runtime.md` § Media).
                let (stored, _held) = store_result(parts, session.blobs()).await;
                report.media = stored;
            }
            Some(report)
        })
    }

    fn end_seen(&self, command: &CommandId) -> bool {
        self.environments.end_seen(command)
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

/// Where a node comes from: created now, or restored from what the tree
/// store holds of it.
pub(crate) enum Origin {
    New {
        record: NodeRecord,
        cwd: String,
        model: ModelSelection,
        /// Its first message, queued in its create commit, so that a node
        /// the process loses before its first save still has it.
        first_message: Option<QueuedMessage>,
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
            Self::New { record, .. } | Self::Stored { record, .. } => record,
        }
    }

    /// The model selection the node infers with: a new node's, or the one
    /// its checkpoint saved.
    pub(crate) fn model(&self) -> &ModelSelection {
        match self {
            Self::New { model, .. } => model,
            Self::Stored { checkpoint, .. } => &checkpoint.state.model,
        }
    }
}

/// What makes a node itself, and what it is built with.
pub(crate) struct NodeSpec<H: HostResolver> {
    pub(crate) origin: Origin,
    pub(crate) role: NodeRole,
    pub(crate) root: NodeId,
    /// A runtime that serves the origin's model selection's provider entry.
    pub(crate) runtime: Box<dyn ProviderRuntime>,
    pub(crate) providers: Rc<dyn ProviderResolver>,
    pub(crate) hosts: Rc<H>,
    pub(crate) instructions: Rc<str>,
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
    } = spec;
    let record = origin.record().clone();
    let session_store = store.session_store(&record.id);
    let cwd = match &origin {
        Origin::New { cwd, .. } => cwd.clone(),
        Origin::Stored { checkpoint, .. } => checkpoint.state.cwd.clone(),
    };
    let index = commands.render_index();
    let node_runtime = Rc::new(NodeRuntime {
        node: record.id.clone(),
        root,
        providers,
        cwd: cwd.clone(),
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
        store: store.clone(),
        shells,
        environments: Environments::default(),
        feed,
    });
    let deps = SessionDeps {
        runtime: node_runtime.clone(),
        store: session_store,
        ids,
        clock,
        config,
    };
    let (session, continuation) = match origin {
        Origin::Stored { checkpoint, .. } => {
            let running = store.running_commands(&record.id).await?;
            let (session, continuation) =
                AgentSession::restore(checkpoint, record.id.clone(), runtime, deps, &running)?;
            (session, Some(continuation))
        }
        Origin::New {
            model,
            first_message,
            ..
        } => {
            let init = SessionInit {
                id: record.id.clone(),
                cwd,
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
            record,
            role,
            session,
            runtime: node_runtime,
        },
        continuation,
    })
}
