//! The core: everything Demi does for one principal's agents, on one
//! thread. The server runs one core per user; a program runs one of its own.
//!
//! A core holds:
//!
//! - the conversations as the agent sees them, their agent trees, idle
//!   watches, schedules, forks and lanes;
//! - the Hosts: the runners connected to it, with their jobs, files,
//!   transfers and pipes;
//! - the plugin host and the product port its plugins ask;
//! - the provider entries, their accounts, model catalogs and media;
//! - the usage ledger and the search index.
//!
//! ```text
//! whoever starts the core: the server, or a program
//!      |  Core::start(user, store, services, starter, extensions)
//!      |  calls on the core's thread       owned data in and out
//!      v
//! Core ---- Starter ------> what only its starter does: notifications,
//!   |                       pairing, shares, the Cloud's machine
//!   |  CoreServices         shared by every core of the process: the
//!   |                       vault, model catalogs, rotation, plugins
//!   v
//! runners, providers, the core store and the conversation stores
//! ```
//!
//! What a core cannot do itself it asks its starter (`Starter`), and what
//! the starter keeps in it for its own work it finds in the core's
//! extensions (`Extensions`). The core never reaches past them.
//!
//! A user's conversations, devices, projects and attachments belong to that
//! user alone in both instance modes. An administrator's role gives no access to
//! another user's conversations or files. Account administration and the shared
//! instance's usage report are the only records the role reaches.
//!
//! # No locks in admission logic
//!
//! A core runs on one thread, so nothing else runs between two of its awaits
//! and admission logic needs no locks: a check and the state change it guards
//! happen with no await between them.
//!
//! A new transfer, for example, checks that the conversation's transfers are not
//! being closed and registers itself in the same synchronous step, so a switch
//! that closes transfers cannot miss it. Such a section carries a comment at the
//! site and a test that interleaves a competitor at that point; an await added
//! inside it reopens the race.
//!
//! # Crossing the boundary
//!
//! The boundary is crossed per request, socket, stream admission or change
//! mark, never per tool call. What crosses is owned data: `Send` pipe ends,
//! leases and marks.
//!
//! - A transfer or user stream returns a lease with its pipe ends. Whoever
//!   asked copies bytes while it holds the lease, and dropping it releases
//!   the admission in the core, which can also end the lease itself, for
//!   example when the conversation is archived.
//! - A change a page shows crosses as a mark in the registry of open
//!   synchronization channels, from whoever committed it. The mark adds the part
//!   to each of the user's channels and wakes their tasks to read it in the
//!   core.
//!
//! # Cancellation
//!
//! A call into a core always runs to completion. When the requester leaves,
//! its token is cancelled and only waits observe it, such as the wait for the
//! file gate or for a Cloud to wake, never a step after a commit: a target
//! switch that has won its storage write still restarts the conversation's idle
//! watch even if the page disconnected.
//!
//! Transitions such as a switch and an archive run as spawned core tasks once
//! they hold the conversation.
//!
//! # Threading
//!
//! Core code uses `Rc`, `RefCell` and local futures, and the traits it calls
//! through (`Host`, `ShellEnvironment`, `RpcHandler`, `ProviderRuntime`) have no
//! `Send` bound; the shared services are `Send + Sync`. Code on another
//! thread reaches a core through `Cores`.
//!
//! # Components
//!
//! Each component owns its state and `Core` combines them, so no component
//! holds a reference back to another. The components are the libraries
//! beneath, which cannot see `Core`.
//!
//! A component whose operations need another's state, such as host access,
//! whose operations hold the user's conversations, defines the narrow trait of
//! what it needs (`HostCore`) and writes its operations as methods of `dyn
//! HostCore`. This crate implements each such trait for `Core`; the server's
//! Cloud implements its own (`CloudCore`) over the core's public methods.
//!
//! Its own behavior is `impl Core` blocks, one per module. The crate serves no
//! HTTP.

mod adoption;
pub(crate) mod agent_profiles;
pub mod conversation;
pub mod cores;
pub(crate) mod credential_commands;
pub mod credentials;
pub mod device_requests;
pub mod extensions;
pub mod holds;
mod host;
pub mod instructions;
pub mod lane;
pub mod lifecycle;
pub mod media;
pub(crate) mod media_jobs;
pub(crate) mod notify_commands;
mod permissions;
mod plugins;
mod policy;
pub mod pool;
pub mod port_forwards;
pub(crate) mod provider_commands;
pub mod provider_settings;
pub mod schedules;
pub mod services;
pub mod settings;
pub mod start_passes;
pub mod starter;
pub mod subagents;
pub mod tuning;
pub mod workspaces;
mod worktrees;

pub use self::adoption::KeptJobs;
pub use self::plugins::{ReloadRefusal, web_client};
pub use self::lane::{Lane, LaneClose, LaneEnd, LaneGone, LaneMessage, LaneReader};

use std::future::Future;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use demi_agent_server::AgentServer;
use demi_core_store::store::CoreStore;
use demi_plugin_host::UserPlugins;
use demi_runner_client::{ARRIVAL, Pipes};
use demi_core_hosts::devices::Devices;
use demi_core_hosts::router::CommandRouter;
use demi_shared_gates::{KeyedSerialGate, SerialGate};
use demi_web_api_protocol::ids::UserId;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::conversation::claude_cli::{ClaudeCli, ClaudeMachine};
use crate::conversation::project_files::ProjectFiles;
use crate::conversation::notifications::Notifications;
use crate::conversation::restart_resume::RestartResumes;
use crate::conversation::retry_resume::RetryResumes;
use crate::conversation::idle_compaction::IdleCompactions;
use crate::conversation::search::SearchIndexer;
use crate::conversation::suggested_replies::SuggestedReplies;
use crate::conversation::titles::Titles;
use crate::conversation::{ConversationParts, CoreHosts};
use crate::lifecycle::collection::BlobCollection;
use crate::lifecycle::conversations::ConversationWatches;
use crate::extensions::Extensions;
use crate::services::CoreServices;
use crate::starter::Starter;
use demi_core_host_access::access::Conversations;
use demi_core_host_access::plugin_files::PluginInstalls;

/// One principal's core: everything Demi decides for that user's agents.
/// Its components arrive with the conversations, devices and Hosts that
/// need them.
pub struct Core {
    user: UserId,
    /// The user's core store.
    store: CoreStore,
    /// The core itself, for the tasks its methods start.
    this: Weak<Core>,
    services: Arc<CoreServices>,
    /// What whoever runs the core gives it.
    starter: Rc<dyn Starter>,
    /// The HTTP client of the core's thread, which the runtimes built here
    /// send with: hyper ties a pooled connection to the runtime that made it.
    http: reqwest::Client,
    /// The user's conversations, each with its file gate and transfers.
    conversations: Conversations,
    /// The user's devices, each with its runner connection.
    devices: Devices,
    /// The pipes of the user's devices.
    pipes: Pipes,
    /// Each agent node's commands, for the rpc calls of its jobs.
    commands: CommandRouter,
    /// Every task the core spawns, which its close waits for.
    tasks: TaskTracker,
    /// The pages' synchronization channels, which the core's close ends
    /// first, so none reads what the close changes.
    channels: TaskTracker,
    /// Cancelled when the core starts closing: it takes no new runner.
    closing: CancellationToken,
    /// The user's conversation trees.
    agent: Rc<AgentServer<CoreHosts>>,
    /// The title requests of the user's conversations.
    titles: Titles,
    /// The suggested replies of the user's conversations.
    suggested_replies: SuggestedReplies,
    /// What keeps the user's search index in step with the conversations.
    search: SearchIndexer,
    /// What opens a conversation's tree: the conversation lanes being
    /// served and the restores of trees with no page attached. The close
    /// ends them before the agent shuts down, so no tree opens and no frame
    /// reaches one after.
    tree_openers: TaskTracker,
    /// The pages' synchronization channels being served, which the close
    /// ends first, so no page is sent what the steps after it change.
    /// Fork requests, one at a time per destination id in lowercase.
    forks: KeyedSerialGate<String>,
    /// The forks and deletions of the user's conversations, one at a time:
    /// a fork holds it from reading its source until its plugins learn of
    /// it, and a deletion from its records' end until its plugins learn of
    /// that, so a plugin never removes what a fork of the deleted
    /// conversation keeps.
    lifecycle: SerialGate,
    /// Workspace deletions, one at a time, so two cannot each find another
    /// workspace left and delete the last's conversations.
    workspace_deletions: SerialGate,
    /// The user's Cloud machine.
    /// Each conversation's idle watch.
    idle_watches: ConversationWatches,
    /// The collections of the user's blob namespace.
    blob_collection: BlobCollection,
    /// The user's Claude Code CLI work, which runs on the Claude Code
    /// machine.
    claude_cli: ClaudeCli,
    /// An instance of every plugin for the user.
    plugins: UserPlugins,
    /// What the core remembers of its Hosts' plugin directories.
    plugin_installs: PluginInstalls,
    /// How many times each conversation's permission requests changed
    /// since the core started.
    permission_revisions: demi_core_sync::Revisions,
    /// The project files its conversations use, kept current by the
    /// runner's file watch.
    project_files: ProjectFiles,
    /// The calls that wait for the answers to their requests.
    waits: demi_core_permissions::Waits,
    /// What rearms the expiry of the user's credentials.
    credential_expiry: credentials::CredentialExpiry,
    /// What the user's runners announced of their forwards, and the peers
    /// introduced between two of them.
    forwards: port_forwards::ForwardsHeld,
    /// What the user's runners were told of their projects, and the store of
    /// what they reported of their worktrees.
    worktrees: worktrees::WorktreesHeld,
    /// The user's schedules' scheduler, change watches and URL calls.
    schedules: crate::schedules::Schedules,
    /// Each conversation's timer for the retry time of its root's failed
    /// turn.
    retry_resumes: RetryResumes,
    /// The conversations whose turn a restart interrupted that it resumes.
    restart_resumes: RestartResumes,
    idle_compactions: IdleCompactions,
    /// The turns its conversations' trees work on, and the agents' own
    /// notifications, for the user's push notifications.
    pub(crate) notifications: Notifications,
    /// The videos' jobs borrowers of the user's lent entries asked for.
    extensions: Extensions,
    /// The media jobs the user's calls asked for, while they run.
    media_jobs: media_jobs::MediaJobs,
}

impl Core {
    /// What this core holds now.
    pub fn census(&self) -> Census {
        let mut census = Census {
            cores: 1,
            runners_online: u64::try_from(self.devices.links().len()).unwrap_or(u64::MAX),
            ..Census::default()
        };
        for root in self.agent.live_roots() {
            let Some(tree) = self.agent.tree(&root) else {
                continue;
            };
            census.live_trees += 1;
            census.live_sessions += u64::try_from(tree.nodes().len()).unwrap_or(u64::MAX);
            census.running_turns += u64::from(tree.works());
            census.trees_running_commands += u64::from(tree.runs_commands());
        }
        census
    }

    /// Starts the core of `user` over `store`: the starter `starter_of`
    /// makes for it, and what the starter keeps in it. The core follows its
    /// changes, schedules and jobs until it closes.
    pub fn start(
        user: UserId,
        store: CoreStore,
        services: Arc<CoreServices>,
        starter_of: impl FnOnce(Weak<Core>) -> Rc<dyn Starter>,
        extensions: Extensions,
        http: reqwest::Client,
        web: reqwest::Client,
    ) -> Rc<Self> {
        let core = Rc::new_cyclic(|core| {
            Core::new(core.clone(), user, store, services, starter_of(core.clone()), extensions, http, web)
        });
        // The search index follows the user's changes from the start until
        // the core closes.
        core.tasks.spawn_local(core.clone().follow_for_search());
        core.tasks.spawn_local(core.clone().follow_search_resets());
        // So does the pages' news of what a call cost.
        core.tasks.spawn_local(core.clone().follow_usage());
        // So do the processes prepared ahead, the changes of
        // the accounts they were started with.
        let changes = core.services.vault.account_changes();
        core
            .tasks
            .spawn_local(core.clone().follow_account_changes(changes));
        // And the account each open conversation's next
        // request takes under its entry's rotation.
        let changes = core.services.provider_registry.rotation().changes();
        core
            .tasks
            .spawn_local(core.clone().follow_rotation_changes(changes));
        // And the expiry of the user's credentials, which
        // removes at once what is past due.
        core
            .tasks
            .spawn_local(core.clone().follow_credential_expiry());
        // And the user's schedules, which fire what a
        // backend that was down missed.
        core
            .tasks
            .spawn_local(core.clone().follow_schedules());
        // And the media jobs an earlier start kept, which
        // end whether or not their calls come back.
        core
            .tasks
            .spawn_local(core.clone().follow_media_jobs());
        core
    }

    /// The core of `user`, whose agent server reaches it through `core`.
    fn new(
        core: Weak<Core>,
        user: UserId,
        store: CoreStore,
        services: Arc<CoreServices>,
        starter: Rc<dyn Starter>,
        extensions: Extensions,
        http: reqwest::Client,
        web: reqwest::Client,
    ) -> Self {
        let plugins = UserPlugins::new(
            services.plugins.clone(),
            user.clone(),
            Rc::new(plugins::CorePort {
                core: core.clone(),
                services: services.clone(),
                key: store.key().clone(),
                web,
            }),
            Rc::new(store.clone()),
            Rc::new(services.sync.of(&user)),
        );
        let ConversationParts { agent, titles, suggested_replies } = conversation::conversation_parts(
            core.clone(),
            user.clone(),
            store.clone(),
            services.clone(),
            http.clone(),
        );
        let devices = Devices::new(
            services.returning.clone(),
            services.runners.returning,
            services.sync.of(&user),
        );
        let project_files = ProjectFiles::new(&core);
        Self {
            search: SearchIndexer::new(&services, &user),
            user,
            store,
            this: core,
            services,
            starter,
            http,
            conversations: Conversations::default(),
            devices,
            pipes: Pipes::new(ARRIVAL),
            commands: CommandRouter::default(),
            tasks: TaskTracker::new(),
            channels: TaskTracker::new(),
            closing: CancellationToken::new(),
            agent,
            titles,
            suggested_replies,
            tree_openers: TaskTracker::new(),
            forks: KeyedSerialGate::new(),
            lifecycle: SerialGate::new(),
            workspace_deletions: SerialGate::new(),
            idle_watches: ConversationWatches::default(),
            blob_collection: BlobCollection::default(),
            claude_cli: ClaudeCli::default(),
            plugins,
            plugin_installs: PluginInstalls::default(),
            permission_revisions: demi_core_sync::Revisions::default(),
            project_files,
            waits: demi_core_permissions::Waits::default(),
            credential_expiry: credentials::CredentialExpiry::default(),
            forwards: port_forwards::ForwardsHeld::default(),
            worktrees: worktrees::WorktreesHeld::default(),
            schedules: crate::schedules::Schedules::default(),
            retry_resumes: RetryResumes::default(),
            restart_resumes: RestartResumes::default(),
            idle_compactions: IdleCompactions::default(),
            notifications: Notifications::default(),
            extensions,
            media_jobs: media_jobs::MediaJobs::default(),
        }
    }

    /// The user's plugins.
    pub fn plugins(&self) -> &UserPlugins {
        &self.plugins
    }

    /// The core, for a task that outlives the call that starts it. A
    /// method of the core runs while its caller holds it.
    pub fn this(&self) -> Rc<Core> {
        self.this
            .upgrade()
            .expect("a core's method runs while the core lives")
    }

    pub(crate) fn claude_cli(&self) -> &ClaudeCli {
        &self.claude_cli
    }

    pub(crate) fn schedules(&self) -> &crate::schedules::Schedules {
        &self.schedules
    }

    pub(crate) fn retry_resumes(&self) -> &RetryResumes {
        &self.retry_resumes
    }

    pub(crate) fn restart_resumes(&self) -> &RestartResumes {
        &self.restart_resumes
    }

    pub(crate) fn idle_compactions(&self) -> &IdleCompactions {
        &self.idle_compactions
    }

    pub(crate) fn idle_watches(&self) -> &ConversationWatches {
        &self.idle_watches
    }

    pub(crate) fn blob_collection(&self) -> &BlobCollection {
        &self.blob_collection
    }

    pub fn user(&self) -> &UserId {
        &self.user
    }

    pub fn services(&self) -> &CoreServices {
        &self.services
    }

    /// What whoever runs the core gives it.
    pub fn starter(&self) -> &dyn Starter {
        self.starter.as_ref()
    }

    /// The user's core store.
    pub fn store(&self) -> &CoreStore {
        &self.store
    }

    /// Marks `part` changed on the user's open channels. A change of the
    /// projects also tells each runner the directories of its Host's
    /// projects, whose worktrees it reports.
    pub fn mark(&self, part: demi_core_sync::Part) {
        let projects = part == demi_core_sync::Part::Projects;
        self.services().sync.mark(self.user(), part);
        if projects {
            let core = self.this();
            self.tasks().spawn_local(async move { core.projects_changed().await });
        }
    }

    /// The value of type `T` whoever started the core keeps in it.
    pub fn extension<T: std::any::Any>(&self) -> Option<&T> {
        self.extensions.get()
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn conversations(&self) -> &Conversations {
        &self.conversations
    }

    pub fn devices(&self) -> &Devices {
        &self.devices
    }

    pub fn pipes(&self) -> &Pipes {
        &self.pipes
    }

    pub fn commands(&self) -> &CommandRouter {
        &self.commands
    }

    pub fn tasks(&self) -> &TaskTracker {
        &self.tasks
    }

    /// The pages' synchronization channels: each holds a token while it
    /// lives.
    pub fn channels(&self) -> &TaskTracker {
        &self.channels
    }

    /// The Claude Code machine, through the master's core; none in a
    /// backend whose cores never started.
    pub(crate) fn claude_machine(&self) -> Option<ClaudeMachine> {
        self.services().claude_machine.get()
    }

    /// Whether the core is closing and starts no new work.
    pub fn is_closing(&self) -> bool {
        self.closing.is_cancelled()
    }

    /// Resolves once the core starts closing.
    pub fn closed(&self) -> tokio_util::sync::WaitForCancellationFuture<'_> {
        self.closing.cancelled()
    }

    pub(crate) fn agent(&self) -> &Rc<AgentServer<CoreHosts>> {
        &self.agent
    }

    pub(crate) fn project_files(&self) -> &ProjectFiles {
        &self.project_files
    }

    pub(crate) fn titles(&self) -> &Titles {
        &self.titles
    }

    pub(crate) fn suggested_replies(&self) -> &SuggestedReplies {
        &self.suggested_replies
    }

    pub(crate) fn search_indexer(&self) -> &SearchIndexer {
        &self.search
    }

    pub(crate) fn tree_openers(&self) -> &TaskTracker {
        &self.tree_openers
    }

    pub fn forks(&self) -> &KeyedSerialGate<String> {
        &self.forks
    }

    pub(crate) fn lifecycle(&self) -> &SerialGate {
        &self.lifecycle
    }

    pub(crate) fn workspace_deletions(&self) -> &SerialGate {
        &self.workspace_deletions
    }

    /// Ends the user's work in order.
    ///
    /// 1. The synchronization channels close.
    /// 2. The idle watches stop, and a retirement already running finishes.
    /// 3. Title requests are aborted.
    /// 4. The conversation lanes end, and so do the restores of trees with no
    ///    page attached.
    /// 5. Open file transfers and user streams end, and stay closed.
    /// 6. The agent turns are aborted, and the commands they watched run on.
    /// 7. A reset or transition of the Cloud finishes, and a running Cloud keeps
    ///    running.
    /// 8. The runner connections close, and then the pipes fail.
    ///
    /// Nothing on a Host stops.
    ///
    /// The order has reasons:
    ///
    /// - The starter's work goes first, such as the pages' channels, so no page
    ///   is sent what the steps below change; a page reads the state again
    ///   from the next backend.
    /// - The lanes end before the agents, so no tree opens and no frame reaches a
    ///   tree after its shutdown.
    /// - A root's running turn is marked in `interrupted_turns` before it is
    ///   aborted, so the next start resumes it however far the shutdown got.
    /// - Transfers and user streams end before the Cloud stops, so the Cloud can
    ///   still save.
    pub async fn close(&self) {
        self.closing.cancel();
        self.channels.close();
        closed("the channels ended", self.channels.wait()).await;
        closed("the starter's work ended", self.starter.closing()).await;
        self.services.sync.forget(self.user());
        self.stop_idle_watches();
        self.retry_resumes.clear();
        self.idle_compactions.clear();
        let cloud = self.starter.cloud();
        if let Some(cloud) = &cloud {
            cloud.stop();
        }
        self.titles.abort_all();
        self.suggested_replies.abort_all();
        self.tree_openers.close();
        closed("the tree openings ended", self.tree_openers.wait()).await;
        let _transfers_closed = closed("the transfers ended", self.conversations.end_transfers()).await;
        closed("the running roots were marked", self.mark_running_roots()).await;
        closed("the agents were disposed", self.agent.shutdown()).await;
        if let Some(cloud) = &cloud {
            closed("the Cloud closed", cloud.close()).await;
        }
        closed("the devices were shut down", self.devices.shut_down(&self.store)).await;
        self.tasks.close();
        closed("the tasks ended", self.tasks.wait()).await;
        closed("the pipes closed", self.pipes.close()).await;
    }
}

/// Awaits `step` of a core's close and logs how long it took, so the journal
/// of a close that hangs names the step after the last it logged.
async fn closed<T>(step: &'static str, work: impl Future<Output = T>) -> T {
    let started = std::time::Instant::now();
    let output = work.await;
    tracing::info!(event = "core_close.step_ended", step, ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX), "a step of the core's close ended");
    output
}

/// What the cores hold now, as the backend's resource sample and its stopping
/// line count it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Census {
    /// The users with a core.
    pub cores: u64,
    /// The conversation trees live in memory.
    pub live_trees: u64,
    /// The agent sessions of those trees, the roots' and their children's.
    pub live_sessions: u64,
    /// The trees that work: a turn runs or waits to run, or a child is live.
    pub running_turns: u64,
    /// The trees with a command running.
    pub trees_running_commands: u64,
    /// The runners connected.
    pub runners_online: u64,
}

impl std::ops::AddAssign for Census {
    fn add_assign(&mut self, other: Self) {
        self.cores += other.cores;
        self.live_trees += other.live_trees;
        self.live_sessions += other.live_sessions;
        self.running_turns += other.running_turns;
        self.trees_running_commands += other.trees_running_commands;
        self.runners_online += other.runners_online;
    }
}
