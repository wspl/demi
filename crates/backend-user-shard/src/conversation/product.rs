//! What the backend's conversations are assembled from (`runtime.md`
//! § Sessions and turns): the product's instructions, the toolset of the
//! plugins the user has on with the product's `demi host` group, the user's
//! subagent settings, which each spawn reads, the Host each
//! node reaches, the conversation's current primary Host, the execution
//! context source, which tells a node that the conversation's execution
//! context changed, and the plugins that are context sources.

use std::rc::{Rc, Weak};

use demi_agent_tools::{
    ContextSource, HostResolver, NodeContext, Profile, ProfileModel, SubagentSettings,
    SubagentSource, Toolset, ToolsetSource,
};
use demi_backend_host_access::host_commands::host_group;
use demi_backend_host_access::{HostShard, conversation_of};
use demi_backend_plugins::ContextAsk;
use demi_backend_remote_host::RemoteHost;
use demi_host_interface::HostError;
use demi_plugin_interface::{EXECUTION_SOURCE, PluginId};
use demi_shared_types::TurnId;
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::shard::Shard;

/// The product's instructions, which open every node's system prompt.
pub(crate) const INSTRUCTIONS: &str = "You are a coding agent. Use shell session tools to inspect, edit, test, and verify the workspace.\n\nTreat cwd as the task workspace. Create, edit, and verify task files there by default; do not create a separate project directory under /tmp or another absolute path unless the user asks for it or the workspace is unusable.";

/// What a conversation's tree opens with: the commands of the plugins the
/// user has on, with the product's `demi host` group.
pub(crate) struct ShardToolsets {
    /// Weak: the shard owns the agent server that holds the source.
    pub(crate) shard: Weak<Shard>,
}

impl ToolsetSource for ShardToolsets {
    fn current(&self) -> LocalBoxFuture<'_, Result<Toolset, String>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| "the backend is shutting down".to_owned())?;
            let hosts: Weak<dyn HostShard> = self.shard.clone();
            let toolset = shard
                .plugins()
                .toolset(vec![host_group(hosts)])
                .await
                .map_err(|error| error.to_string())?;
            Ok(Toolset {
                commands: Rc::new(toolset.commands),
                revision: Rc::from(toolset.revision),
            })
        })
    }
}

/// The user's subagent settings as the control store holds them now.
pub(crate) struct ShardSubagents {
    /// Weak: the shard owns the agent server that holds the source.
    pub(crate) shard: Weak<Shard>,
}

impl SubagentSource for ShardSubagents {
    fn current(&self) -> LocalBoxFuture<'_, Result<SubagentSettings, String>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| "the backend is shutting down".to_owned())?;
            let settings = shard
                .services()
                .control
                .subagent_settings(shard.user().clone())
                .await
                .map_err(|error| error.to_string())?;
            let profiles = settings
                .profiles
                .into_iter()
                .map(|profile| Profile {
                    name: profile.name,
                    description: profile.description,
                    model: profile.model.map(|model| ProfileModel {
                        provider_id: model.provider_id.as_str().to_owned(),
                        model_id: model.model_id,
                        thinking_effort: model.thinking_effort,
                        service_tier_id: model.service_tier_id,
                    }),
                    instructions: profile.instructions,
                    can_spawn: profile.can_spawn,
                    enabled: profile.enabled,
                })
                .collect();
            Ok(SubagentSettings {
                enabled: settings.enabled,
                profiles,
            })
        })
    }
}

/// Where a shard's conversations run: each conversation's current primary Host.
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
        shard
            .host_shard()
            .conversation_host(&conversation_of(context.root))
            .await
    }
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
    ) -> LocalBoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(async move {
            let Some(shard) = self.shard.upgrade() else {
                return Ok(None);
            };
            shard
                .execution_context(&conversation_of(node.root), seen)
                .await
                .map_err(|error| format!("the execution context cannot be read: {error}"))
        })
    }
}

/// A plugin that is a context source, asked while its user has it on
/// (`plugins.md` § Prompt text and context).
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
    ) -> LocalBoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(async move {
            let Some(shard) = self.shard.upgrade() else {
                return Ok(None);
            };
            let asked = ContextAsk {
                conversation: conversation_of(node.root),
                node: node.node.clone(),
                cwd: node.cwd.to_owned(),
                turn: turn.clone(),
                seen: seen.iter().map(|text| (*text).to_owned()).collect(),
            };
            // The request ends with the provider request it serves, which
            // drops this future.
            shard
                .plugins()
                .context(&self.plugin, asked, CancellationToken::new())
                .await
                .map_err(|error| format!("the plugin {} gave no context: {error}", self.plugin))
        })
    }
}
