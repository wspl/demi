//! What the backend's conversations are assembled from.
//!
//! - The product's instructions.
//! - The toolset of the plugins the user has on, with the product's `demi host`
//!   and `demi attachment` groups.
//! - The user's subagent settings, which each spawn reads.
//! - The Host each node reaches, and the conversation's current primary Host.
//! - The execution context source, which tells a node that the conversation's
//!   execution context changed.
//! - The plugins that are context sources.

use std::rc::{Rc, Weak};

use demi_agent_tools::{
    ContextAnswer, ContextSource, ConversationProfiles, HostResolver, NewProfileDraft, NodeContext, Profile, WorkflowListing,
    ProfileEdit, ProfileModel, SubagentSettings, SubagentSource, Toolset, ToolsetSource,
    runs_on as job_model,
};
use demi_conversation_lane_protocol::{CacheChart, JobModel};
use demi_shared_types::{IntegrationCapabilities, ModelSelection, NodeId, PictureFailure, SKILLS_SOURCE, WorkflowChoice};

use super::runs_on::runs_on;
use demi_backend_host_access::attachment_commands::attachment_group;
use demi_backend_host_access::host_commands::host_group;

use super::organize::conversation_group;
use crate::shard::provider_commands::providers_group;
use demi_backend_host_access::{HostShard, conversation_of};
use demi_plugin_host::ContextAsk;
use demi_runner_client::RemoteHost;
use demi_agent_server::GuideSource;
use demi_agent_store::Placement;
use demi_web_api_protocol::ids::DeviceId;
use demi_host_interface::{HostError, HostErrorKind};
use demi_plugin_interface::{EXECUTION_SOURCE, PluginId};
use demi_shared_types::TurnId;
use demi_backend_database::StorageError;
use demi_provider_registry::entry::ProviderEntry;
use demi_web_api_protocol::subagents::SubagentProfile;
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::shard::Shard;
use crate::schedules::commands::schedule_group;
use crate::shard::credential_commands::credentials_group;
use crate::shard::notify_commands::notify_group;

/// The product's instructions, the identity that opens every node's system
/// prompt, which a subagent profile's replace.
pub(crate) const INSTRUCTIONS: &str = "You are Demi, an agent that works for the user on their computers: a Demi Cloud machine or the devices they paired. You write and run software, research, operate websites, and work with files and data, across turns.";

/// The rules of the runtime every agent reads, in its harness guide: where
/// its commands run and how it waits.
const RUNTIME_RULES: &str = "- Commands run on the conversation's primary Host, in its working directory; a context block names both. The directory stays between turns, and the work panel shows it.\n- To wait for a command or a subagent, end your turn: its end or its message wakes you.";

/// What the web app shows of a reply, which an agent of the user's own
/// conversation reads after the rules: an integration shows replies in its
/// own way, and says so in its workflow's instructions.
const DISPLAY_CONVENTIONS: &str = "- The user reads only your replies, rendered as Markdown. Be concise.\n- Deliver a file as an attachment: `demi attachment upload`, then `<attachment id=\"a3\" />` on its own line; a medium a tool result showed you has its `attachment_id` already.\n- Link what the user can open: a Host file `[runner.md](docs/runner.md#L438)`, a subagent `[File icons](agent:4)`, a commit `[Fix the cart total](commit:3f2a9c1)`, or `commit:src/v8@86229da` in another repository.";

/// The harness guide of a conversation's agents: the runtime's rules, and for
/// the user's own conversation the web app's display conventions.
pub(crate) struct ShardGuides {
    pub(crate) shard: Weak<Shard>,
}

impl GuideSource for ShardGuides {
    fn guide<'a>(&'a self, root: &'a NodeId) -> LocalBoxFuture<'a, Rc<str>> {
        Box::pin(async move {
            let integration = match self.shard.upgrade() {
                Some(shard) => shard.integration_capabilities(Some(root)).await.ok().flatten().is_some(),
                None => false,
            };
            Rc::from(if integration {
                format!("Guidelines:\n{RUNTIME_RULES}")
            } else {
                format!("Guidelines:\n{RUNTIME_RULES}\n{DISPLAY_CONVENTIONS}")
            })
        })
    }
}

/// What a conversation's tree opens with: the commands of the plugins the
/// user has on, with the product's `demi host`, `demi attachment`, `demi
/// conversation`, `demi credentials`, `demi providers` and `demi notify`
/// groups.
pub(crate) struct ShardToolsets {
    /// Weak: the shard owns the agent server that holds the source.
    pub(crate) shard: Weak<Shard>,
}

impl ToolsetSource for ShardToolsets {
    fn revision<'a>(&'a self, root: Option<&'a NodeId>) -> LocalBoxFuture<'a, Result<Rc<str>, String>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| "the backend is shutting down".to_owned())?;
            let revision = match shard.integration_capabilities(root).await? {
                Some(capabilities) => {
                    let plugins = shard
                        .plugins()
                        .revision_where(|plugin| integration_commands(&capabilities, plugin))
                        .await
                        .map_err(|error| error.to_string())?;
                    format!("integration {} {plugins}", capability_bits(&capabilities))
                }
                None => shard.plugins().revision().await.map_err(|error| error.to_string())?,
            };
            Ok(Rc::from(revision))
        })
    }

    fn current<'a>(&'a self, root: Option<&'a NodeId>) -> LocalBoxFuture<'a, Result<Toolset, String>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| "the backend is shutting down".to_owned())?;
            let hosts: Weak<dyn HostShard> = self.shard.clone();
            let toolset = match shard.integration_capabilities(root).await? {
                // An integration's agents have no Hosts of their own to name,
                // no credentials, notifications, other conversations or the
                // user's providers to manage, and the groups and plugins of
                // their capabilities alone.
                Some(capabilities) => {
                    let mut groups = Vec::new();
                    if capabilities.attachments {
                        groups.push(attachment_group(hosts));
                    }
                    if capabilities.schedules {
                        groups.push(schedule_group(self.shard.clone()));
                    }
                    let toolset = shard
                        .plugins()
                        .toolset_where(groups, |plugin| integration_commands(&capabilities, plugin))
                        .await
                        .map_err(|error| error.to_string())?;
                    Toolset {
                        commands: Rc::new(toolset.commands),
                        revision: Rc::from(format!("integration {} {}", capability_bits(&capabilities), toolset.revision)),
                    }
                }
                None => {
                    let toolset = shard
                        .plugins()
                        .toolset(vec![
                            host_group(hosts.clone()),
                            attachment_group(hosts),
                            conversation_group(self.shard.clone()),
                            credentials_group(self.shard.clone()),
                            schedule_group(self.shard.clone()),
                            providers_group(self.shard.clone()),
                            notify_group(self.shard.clone()),
                        ])
                        .await
                        .map_err(|error| error.to_string())?;
                    Toolset {
                        commands: Rc::new(toolset.commands),
                        revision: Rc::from(toolset.revision),
                    }
                }
            };
            Ok(toolset)
        })
    }
}

/// Whether an integration with `capabilities` has the plugin `plugin`: the
/// file plugins always, the others as their capability says.
pub(crate) fn integration_allows(capabilities: &IntegrationCapabilities, plugin: &str) -> bool {
    match plugin {
        plugin if plugin == SKILLS_SOURCE => capabilities.skills,
        "browser" => capabilities.browser,
        "web" => capabilities.web,
        "image" | "video" | "audio" => capabilities.media,
        "mobile" => capabilities.mobile,
        _ => true,
    }
}

/// Whether an integration with `capabilities` has the commands of the plugin
/// `plugin`: as [`integration_allows`] says, and never the skills plugin's,
/// which manage the user's own skill sources; its skills reach the
/// integration's agents as context.
fn integration_commands(capabilities: &IntegrationCapabilities, plugin: &str) -> bool {
    plugin != SKILLS_SOURCE && integration_allows(capabilities, plugin)
}

/// The capabilities as a revision names them, one digit each in their order.
fn capability_bits(capabilities: &IntegrationCapabilities) -> String {
    [
        capabilities.subagents,
        capabilities.browser,
        capabilities.web,
        capabilities.skills,
        capabilities.media,
        capabilities.mobile,
        capabilities.schedules,
        capabilities.attachments,
    ]
    .iter()
    .map(|on| if *on { '1' } else { '0' })
    .collect()
}

/// The user's subagent settings as the control store holds them now, and
/// the changes `demi agent profiles` makes to them through the settings
/// page's path.
pub(crate) struct ShardSubagents {
    /// Weak: the shard owns the agent server that holds the source.
    pub(crate) shard: Weak<Shard>,
}

impl ShardSubagents {
    fn shard(&self) -> Result<Rc<Shard>, String> {
        self.shard
            .upgrade()
            .ok_or_else(|| "the backend is shutting down".to_owned())
    }
}

impl SubagentSource for ShardSubagents {
    fn current(&self) -> LocalBoxFuture<'_, Result<SubagentSettings, String>> {
        Box::pin(async move {
            let shard = self.shard()?;
            let settings = shard
                .services()
                .control
                .subagent_settings(shard.user().clone())
                .await
                .map_err(|error| error.to_string())?;
            let entries = entries(&shard).await.map_err(|error| error.to_string())?;
            let profiles = settings
                .profiles
                .into_iter()
                .map(|profile| profile_of(profile, &entries))
                .collect();
            Ok(SubagentSettings {
                enabled: settings.enabled,
                max_live: settings.max_live.bound(),
                max_depth: settings.max_depth.bound(),
                profiles,
            })
        })
    }

    fn runs_on<'a>(&'a self, selections: &'a [ModelSelection]) -> LocalBoxFuture<'a, Vec<JobModel>> {
        Box::pin(async move {
            let Ok(shard) = self.shard() else {
                // The backend is shutting down: the frames name what the
                // selections hold.
                return selections
                    .iter()
                    .map(|selection| job_model(selection, None, None))
                    .collect();
            };
            runs_on(shard.services(), shard.user(), selections).await
        })
    }

    fn create_profile(
        &self,
        profile: NewProfileDraft,
    ) -> LocalBoxFuture<'_, Result<Profile, String>> {
        Box::pin(async move {
            let shard = self.shard()?;
            let created = shard.create_drafted_profile(profile).await?;
            Ok(answered(&shard, created).await)
        })
    }

    fn edit_profile<'a>(
        &'a self,
        name: &'a str,
        edit: ProfileEdit,
    ) -> LocalBoxFuture<'a, Result<Profile, String>> {
        Box::pin(async move {
            let shard = self.shard()?;
            let changed = shard.edit_named_profile(name, edit).await?;
            Ok(answered(&shard, changed).await)
        })
    }

    fn delete_profile<'a>(&'a self, name: &'a str) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move { self.shard()?.delete_named_profile(name).await })
    }

    fn conversation_profiles<'a>(
        &'a self,
        root: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<ConversationProfiles, String>> {
        Box::pin(async move {
            self.shard()?
                .conversation_profiles(&conversation_of(root))
                .await
        })
    }

    fn reaches_user_profiles<'a>(&'a self, root: &'a NodeId) -> LocalBoxFuture<'a, bool> {
        Box::pin(async move {
            match self.shard() {
                Ok(shard) => shard.integration_capabilities(Some(root)).await.is_ok_and(|capabilities| capabilities.is_none()),
                Err(_) => false,
            }
        })
    }

    fn workflows<'a>(&'a self, root: &'a NodeId) -> LocalBoxFuture<'a, Result<WorkflowListing, String>> {
        Box::pin(async move { self.shard()?.workflow_listing(&conversation_of(root)).await })
    }

    fn choose_workflow<'a>(
        &'a self,
        root: &'a NodeId,
        choice: WorkflowChoice,
    ) -> LocalBoxFuture<'a, Result<(WorkflowChoice, Option<String>), String>> {
        Box::pin(async move {
            let shard = self.shard()?;
            // An integration's conversation spawns by the workflow it was
            // made with.
            if shard.integration_capabilities(Some(root)).await?.is_some() {
                return Err("this conversation's workflow is the one its integration made it with".to_owned());
            }
            shard.choose_workflow(&conversation_of(root), choice).await
        })
    }

    fn read_workflow_file<'a>(
        &'a self,
        root: &'a NodeId,
        directory: &'a str,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            self.shard()?
                .workflow_file_text(&conversation_of(root), directory, path)
                .await
        })
    }
}

/// The provider entries in the user's scope.
pub(crate) async fn entries(shard: &Shard) -> Result<Vec<ProviderEntry>, StorageError> {
    shard.services().vault.entries_for(shard.user()).await
}

/// The profile a change wrote, as the agent reads it.
async fn answered(shard: &Shard, profile: SubagentProfile) -> Profile {
    // The write is done: entries that cannot be read only leave the entry's
    // name out of the answer.
    let entries = entries(shard).await.unwrap_or_default();
    profile_of(profile, &entries)
}

/// A stored profile as the agent reads it, with its entry's name among
/// `entries`.
fn profile_of(profile: SubagentProfile, entries: &[ProviderEntry]) -> Profile {
    Profile {
        name: profile.name,
        description: profile.description,
        model: profile.model.map(|model| ProfileModel {
            provider_name: entries
                .iter()
                .find(|entry| entry.id == model.provider_id)
                .map(|entry| entry.label.clone()),
            provider_id: model.provider_id.as_str().to_owned(),
            model_id: model.model_id,
            thinking_effort: model.thinking_effort,
            service_tier_id: model.service_tier_id,
        }),
        context_limit: profile.context_limit,
        instructions: profile.instructions,
        can_spawn: profile.can_spawn,
        enabled: profile.enabled,
    }
}

/// Where a shard's agents run: a subagent's own Host, and each conversation's
/// current primary Host for its root.
pub(crate) struct ShardHosts {
    /// Weak: the shard owns the agent server that holds the resolver.
    pub(crate) shard: Weak<Shard>,
}

impl HostResolver for ShardHosts {
    type Host = RemoteHost;

    async fn host(&self, context: NodeContext<'_>) -> Result<Rc<RemoteHost>, HostError> {
        let shard = self
            .shard
            .upgrade()
            .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
        let conversation = conversation_of(context.root);
        match agent_device(context.placement) {
            Some(device) => shard.host_shard().agent_host(&conversation, &device).await,
            None => shard.host_shard().conversation_host(&conversation).await,
        }
    }

    async fn adopted_host(&self, context: NodeContext<'_>) -> Result<Rc<RemoteHost>, HostError> {
        let shard = self
            .shard
            .upgrade()
            .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
        let conversation = conversation_of(context.root);
        match agent_device(context.placement) {
            Some(device) => shard.host_shard().adopting_agent_host(&conversation, &device).await,
            None => shard.host_shard().adopting_host(&conversation).await,
        }
    }

    async fn host_name(&self, device: &str) -> Option<String> {
        let shard = self.shard.upgrade()?;
        let device = DeviceId::try_from(device.to_owned()).ok()?;
        shard.services().control.device(device).await.ok().flatten().map(|record| record.name)
    }

    /// The conversation's target, read without reaching the Host.
    async fn placement(&self, context: NodeContext<'_>) -> Result<Placement, HostError> {
        let shard = self
            .shard
            .upgrade()
            .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
        shard
            .placement(&conversation_of(context.root))
            .await
            .map_err(|error| HostError::new(HostErrorKind::Failed { code: None }, error.to_string()))
    }
}

/// The device of a subagent's own Host, by its `placement`; none for the
/// root, and for a subagent on the user's Cloud not made yet, which works
/// where the conversation does until it is made.
fn agent_device(placement: Option<&Placement>) -> Option<DeviceId> {
    let device = placement?.device.clone()?;
    DeviceId::try_from(device).ok()
}

/// The context source that announces a change of the conversation's
/// execution context: a target switch, attached hosts, a Cloud reset.
pub(crate) struct ExecutionContext {
    pub(crate) shard: Weak<Shard>,
}

impl ContextSource for ExecutionContext {
    fn name(&self) -> &str {
        EXECUTION_SOURCE
    }

    fn context<'a>(
        &'a self,
        node: NodeContext<'a>,
        _turn: &'a TurnId,
        seen: &'a [&'a str],
    ) -> LocalBoxFuture<'a, Result<Option<ContextAnswer>, String>> {
        Box::pin(async move {
            let Some(shard) = self.shard.upgrade() else {
                return Ok(None);
            };
            shard
                .execution_context(&conversation_of(node.root), seen)
                .await
                .map(|text| text.map(ContextAnswer::from))
                .map_err(|error| format!("the execution context cannot be read: {error}"))
        })
    }
}

/// A plugin that is a context source, asked while its user has it on
/// (`crates/plugin-interface/src/manifest.rs`).
pub(crate) struct PluginContext {
    pub(crate) shard: Weak<Shard>,
    pub(crate) plugin: PluginId,
}

impl ContextSource for PluginContext {
    fn name(&self) -> &str {
        self.plugin.as_str()
    }

    fn context<'a>(
        &'a self,
        node: NodeContext<'a>,
        turn: &'a TurnId,
        seen: &'a [&'a str],
    ) -> LocalBoxFuture<'a, Result<Option<ContextAnswer>, String>> {
        Box::pin(async move {
            let Some(shard) = self.shard.upgrade() else {
                return Ok(None);
            };
            // An integration's agents hear the plugins its capabilities
            // allow.
            if let Some(capabilities) = shard.integration_capabilities(Some(node.root)).await?
                && !integration_allows(&capabilities, self.plugin.as_str())
            {
                return Ok(None);
            }
            let conversation = conversation_of(node.root);
            let paths = shard
                .host_paths(&conversation)
                .await
                .map_err(|error| format!("the plugin {} was not asked: {error}", self.plugin))?;
            let cwd = shard
                .node_placement(node)
                .await
                .map_err(|error| format!("the plugin {} was not asked: {error}", self.plugin))?
                .directory;
            // A plugin the user has off keeps nothing, and is not asked.
            let searches = shard
                .plugins()
                .searches_projects(&self.plugin)
                .await
                .map_err(|error| format!("the plugin {} was not asked: {error}", self.plugin))?;
            let project = if searches {
                shard.kept_project_search(&self.plugin, node.root).await
            } else {
                None
            };
            let asked = ContextAsk {
                conversation,
                node: node.node.clone(),
                cwd,
                paths,
                turn: turn.clone(),
                seen: seen.iter().map(|text| (*text).to_owned()).collect(),
                project,
            };
            // The request ends with the provider request it serves, which
            // drops this future.
            shard
                .plugins()
                .context(&self.plugin, asked, CancellationToken::new())
                .await
                .map(|text| text.map(ContextAnswer::from))
                .map_err(|error| format!("the plugin {} gave no context: {error}", self.plugin))
        })
    }
}

/// What the backend keeps of a conversation's agents that the tree shows:
/// each agent's cost and, read apart, its cache chart, from the usage ledger
/// (`crates/backend-database/src/usage.rs`), and each side question's card.
pub(crate) struct ShardAgents {
    /// Weak: the shard owns the agent server that holds the source.
    pub(crate) shard: Weak<Shard>,
}

impl demi_agent_server::AgentsSource for ShardAgents {
    fn kept<'a>(&'a self, root: &'a NodeId) -> LocalBoxFuture<'a, demi_agent_server::KeptAgents> {
        Box::pin(async move {
            let mut kept = demi_agent_server::KeptAgents::default();
            let Some(shard) = self.shard.upgrade() else {
                return kept;
            };
            let conversation = demi_backend_host_access::conversation_of(root);
            let services = shard.services();
            match services.control.conversation_costs(conversation.clone()).await {
                Ok(costs) => {
                    kept.costs = costs
                        .into_iter()
                        .filter_map(|kept| Some((kept.agent?, kept.cost)))
                        .collect();
                }
                // The entries show no cost until the agent's next row.
                Err(error) => tracing::warn!(
                    event = "conversation_state.read_failed",
                    %conversation,
                    what = "kept_costs",
                    error = &error as &dyn std::error::Error,
                    "a conversation's kept costs were not read"
                ),
            }
            match services
                .conversations
                .read(&conversation, demi_backend_database::side_questions::cards)
                .await
            {
                Ok(cards) => {
                    kept.cards = cards
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(id, title, folded)| (id, demi_agent_server::SideCard { title, folded }))
                        .collect();
                }
                // The cards show untitled and open until they change.
                Err(error) => tracing::warn!(
                    event = "conversation_state.read_failed",
                    %conversation,
                    what = "side_cards",
                    error = &error as &dyn std::error::Error,
                    "a conversation's side question cards were not read"
                ),
            }
            kept
        })
    }

    fn charts<'a>(
        &'a self,
        root: &'a NodeId,
    ) -> LocalBoxFuture<'a, std::collections::HashMap<u64, CacheChart>> {
        Box::pin(async move {
            let Some(shard) = self.shard.upgrade() else {
                return std::collections::HashMap::new();
            };
            let conversation = demi_backend_host_access::conversation_of(root);
            match shard.services().control.conversation_charts(conversation.clone()).await {
                Ok(charts) => charts.into_iter().collect(),
                // The cards show no rate and no chart until the agent's next
                // turn call.
                Err(error) => {
                    tracing::warn!(
                        event = "conversation_state.read_failed",
                        %conversation,
                        what = "cache_charts",
                        error = &error as &dyn std::error::Error,
                        "a conversation's cache charts were not read"
                    );
                    std::collections::HashMap::new()
                }
            }
        })
    }
}

/// Which paths a complete reply names are files on its conversation's primary
/// Host: one look for them all, admitted as a user's look is, which never
/// wakes a stopped Cloud (`crates/backend-host-access/src/stream.rs`).
pub(crate) struct ShardReplyFiles {
    /// Weak: the shard owns the agent server that holds the source.
    pub(crate) shard: Weak<Shard>,
}

impl demi_agent_server::ReplyFiles for ShardReplyFiles {
    fn files<'a>(
        &'a self,
        root: &'a NodeId,
        cwd: &'a str,
        paths: Vec<String>,
    ) -> LocalBoxFuture<'a, Option<std::collections::HashMap<String, String>>> {
        Box::pin(async move {
            let shard = self.shard.upgrade()?;
            let conversation = conversation_of(root);
            let host_paths = match shard.host_paths(&conversation).await {
                Ok(host_paths) => host_paths,
                Err(error) => {
                    tracing::warn!(event = "conversation_state.read_failed", %conversation, what = "reply_paths", error = &error as &dyn std::error::Error, "a reply's code paths were not looked at");
                    return None;
                }
            };
            let resolved: Vec<(String, String)> = paths
                .into_iter()
                .map(|path| {
                    let on_host = host_paths.resolve(cwd, &path);
                    (path, on_host)
                })
                .collect();
            // Released with the shard's tasks' cancellation, which a look
            // needs only to end its wait.
            let cancel = tokio_util::sync::CancellationToken::new();
            let looked = shard
                .host_shard()
                .look_at_host(&conversation, &cancel, async move |host| {
                    use demi_host_interface::HostFs as _;
                    let stats = futures_util::future::join_all(
                        resolved.iter().map(|(_, on_host)| host.host.stat(on_host.as_str())),
                    )
                    .await;
                    resolved
                        .into_iter()
                        .zip(stats)
                        .filter(|(_, stat)| {
                            matches!(stat, Ok(demi_host_interface::FileStat { kind: demi_host_interface::FileKind::File, .. }))
                        })
                        .map(|(paths, _)| paths)
                        .collect::<std::collections::HashMap<String, String>>()
                })
                .await;
            match looked {
                Ok(files) => Some(files),
                // A stopped Cloud or an offline device cannot say; the reply's
                // code stays code.
                Err(error) => {
                    tracing::warn!(event = "conversation_state.read_failed", %conversation, what = "reply_paths", %error, "a reply's code paths could not be looked at");
                    None
                }
            }
        })
    }

    fn copy_picture<'a>(
        &'a self,
        root: &'a NodeId,
        node: &'a NodeId,
        device: Option<&'a str>,
        cwd: &'a str,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, PictureFailure>> {
        Box::pin(async move {
            // The backend closing, a conversation with no Host yet, a device
            // that is no device and a home not known are each a Host the
            // picture cannot be copied from.
            let shard = self.shard.upgrade().ok_or(PictureFailure::HostUnavailable)?;
            let conversation = conversation_of(root);
            let device = device
                .and_then(|device| demi_web_api_protocol::ids::DeviceId::try_from(device).ok())
                .ok_or(PictureFailure::HostUnavailable)?;
            // A path from the Host's home is read from the home its runner
            // reported.
            let path = match path.strip_prefix("~/") {
                Some(rest) => {
                    let home = shard.devices().home(&device).ok_or(PictureFailure::HostUnavailable)?;
                    shard
                        .host_paths(&conversation)
                        .await
                        .map_err(|_| PictureFailure::HostUnavailable)?
                        .join(&home, rest)
                }
                None => path.to_owned(),
            };
            demi_backend_host_access::attachment_commands::copy_picture(
                shard.host_shard(),
                &conversation,
                &device,
                cwd,
                node.clone(),
                &path,
            )
            .await
        })
    }
}
