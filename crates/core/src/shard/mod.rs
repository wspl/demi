//! The core: one principal's conversations, Hosts, plugins, providers,
//! schedules and media, on one thread. Whoever starts it gives it its
//! store, the services it shares with other cores, a starter and what the
//! starter keeps in it.

mod adoption;
pub(crate) mod agent_profiles;
pub(crate) mod credential_commands;
pub(crate) mod notify_commands;
pub mod credentials;
pub mod cores;
pub mod holds;
pub mod instructions;
mod host;
pub mod lane;
mod permissions;
pub mod media;
pub(crate) mod media_jobs;
pub(crate) mod provider_commands;

pub use self::adoption::KeptJobs;
mod plugins;

pub use self::plugins::{ReloadRefusal, web_client};
pub mod device_requests;
mod policy;
pub mod port_forwards;
pub mod subagents;
mod worktrees;
pub mod workspaces;

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
use crate::conversation::{self, ConversationParts, ShardHosts};
use crate::lifecycle::collection::BlobCollection;
use crate::lifecycle::conversations::ConversationWatches;
use crate::extensions::Extensions;
use crate::services::CoreServices;
use crate::starter::Starter;
use demi_core_host_access::access::Conversations;
use demi_core_host_access::plugin_files::PluginInstalls;

/// One user's shard: everything the backend decides for that user. Its
/// components arrive with the conversations, devices and Cloud that need
/// them.
pub struct Shard {
    user: UserId,
    /// The user's core store.
    store: CoreStore,
    /// The shard itself, for the tasks its methods start.
    this: Weak<Shard>,
    services: Arc<CoreServices>,
    /// What whoever runs the core gives it.
    starter: Rc<dyn Starter>,
    /// The HTTP client of the shard's thread, which the runtimes built here
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
    /// Every task the shard spawns, which its close waits for.
    tasks: TaskTracker,
    /// Cancelled when the shard starts closing: it takes no new runner.
    closing: CancellationToken,
    /// The user's conversation trees.
    agent: Rc<AgentServer<ShardHosts>>,
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
    /// What the shard remembers of its Hosts' plugin directories.
    plugin_installs: PluginInstalls,
    /// How many times each conversation's permission requests changed
    /// since the shard started.
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

impl Shard {
    /// What this shard holds now.
    pub fn census(&self) -> Census {
        let mut census = Census {
            shards: 1,
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
        starter_of: impl FnOnce(Weak<Shard>) -> Rc<dyn Starter>,
        extensions: Extensions,
        http: reqwest::Client,
        web: reqwest::Client,
    ) -> Rc<Self> {
        let shard = Rc::new_cyclic(|shard| {
            Shard::new(shard.clone(), user, store, services, starter_of(shard.clone()), extensions, http, web)
        });
        // The search index follows the user's changes from the start until
        // the core closes.
        shard.tasks.spawn_local(shard.clone().follow_for_search());
        shard.tasks.spawn_local(shard.clone().follow_search_resets());
        // So does the pages' news of what a call cost.
        shard.tasks.spawn_local(shard.clone().follow_usage());
        // So do the processes prepared ahead, the changes of
        // the accounts they were started with.
        let changes = shard.services.vault.account_changes();
        shard
            .tasks
            .spawn_local(shard.clone().follow_account_changes(changes));
        // And the account each open conversation's next
        // request takes under its entry's rotation.
        let changes = shard.services.provider_registry.rotation().changes();
        shard
            .tasks
            .spawn_local(shard.clone().follow_rotation_changes(changes));
        // And the expiry of the user's credentials, which
        // removes at once what is past due.
        shard
            .tasks
            .spawn_local(shard.clone().follow_credential_expiry());
        // And the user's schedules, which fire what a
        // backend that was down missed.
        shard
            .tasks
            .spawn_local(shard.clone().follow_schedules());
        // And the media jobs an earlier start kept, which
        // end whether or not their calls come back.
        shard
            .tasks
            .spawn_local(shard.clone().follow_media_jobs());
        shard
    }

    /// The shard of `user`, whose agent server reaches it through `shard`.
    fn new(
        shard: Weak<Shard>,
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
            Rc::new(plugins::ShardPort {
                shard: shard.clone(),
                services: services.clone(),
                web,
            }),
            Rc::new(store.clone()),
            Rc::new(services.sync.of(&user)),
        );
        let ConversationParts { agent, titles, suggested_replies } = conversation::conversation_parts(
            shard.clone(),
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
        let project_files = ProjectFiles::new(&shard);
        Self {
            search: SearchIndexer::new(&services, &user),
            user,
            store,
            this: shard,
            services,
            starter,
            http,
            conversations: Conversations::default(),
            devices,
            pipes: Pipes::new(ARRIVAL),
            commands: CommandRouter::default(),
            tasks: TaskTracker::new(),
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

    /// The shard, for a task that outlives the call that starts it. A
    /// method of the shard runs while its caller holds it.
    pub fn this(&self) -> Rc<Shard> {
        self.this
            .upgrade()
            .expect("a shard's method runs while the shard lives")
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
            let shard = self.this();
            self.tasks().spawn_local(async move { shard.projects_changed().await });
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

    /// The Claude Code machine, through the master's shard; none in a
    /// backend whose shards never started.
    pub(crate) fn claude_machine(&self) -> Option<ClaudeMachine> {
        self.services().claude_machine.get()
    }

    /// Whether the shard is closing and starts no new work.
    pub fn is_closing(&self) -> bool {
        self.closing.is_cancelled()
    }

    /// Resolves once the shard starts closing.
    pub fn closed(&self) -> tokio_util::sync::WaitForCancellationFuture<'_> {
        self.closing.cancelled()
    }

    pub(crate) fn agent(&self) -> &Rc<AgentServer<ShardHosts>> {
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

/// Awaits `step` of a shard's close and logs how long it took, so the journal
/// of a close that hangs names the step after the last it logged.
async fn closed<T>(step: &'static str, work: impl Future<Output = T>) -> T {
    let started = std::time::Instant::now();
    let output = work.await;
    tracing::info!(event = "shard_close.step_ended", step, ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX), "a step of the shard's close ended");
    output
}

/// What the shards hold now, as the backend's resource sample and its stopping
/// line count it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Census {
    /// The users with a shard.
    pub shards: u64,
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
        self.shards += other.shards;
        self.live_trees += other.live_trees;
        self.live_sessions += other.live_sessions;
        self.running_turns += other.running_turns;
        self.trees_running_commands += other.trees_running_commands;
        self.runners_online += other.runners_online;
    }
}
