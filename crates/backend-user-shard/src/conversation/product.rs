//! What the backend's conversations are assembled from (`runtime.md`
//! § Sessions and turns): the product's instructions, the toolset of the
//! plugins the user has on with the product's `demi host` and `demi
//! attachment` groups, the user's
//! subagent settings, which each spawn reads, the Host each
//! node reaches, the conversation's current primary Host, the execution
//! context source, which tells a node that the conversation's execution
//! context changed, and the plugins that are context sources.

use std::rc::{Rc, Weak};

use demi_agent_tools::{
    ContextAnswer, ContextSource, HostResolver, HostWait, NodeContext, Profile, ProfileModel, SubagentSettings,
    SubagentSource, Toolset, ToolsetSource,
};
use demi_backend_host_access::attachment_commands::attachment_group;
use demi_backend_host_access::host_commands::host_group;

use super::organize::conversation_group;
use demi_backend_host_access::{HostShard, conversation_of};
use demi_backend_plugins::ContextAsk;
use demi_backend_remote_host::RemoteHost;
use demi_host_interface::HostError;
use demi_plugin_interface::{EXECUTION_SOURCE, PluginId};
use demi_shared_types::TurnId;
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::shard::Shard;

/// The product's instructions, the identity that opens every node's system
/// prompt, which a subagent profile's replace (`system-prompt.md` § Identity).
pub(crate) const INSTRUCTIONS: &str = "You are Demi, an agent that does work for the user on their computers: a Demi Cloud machine or devices the user paired with Demi. You can do what a capable person at a terminal can: write and run software, research, operate websites, work with files and data, and carry long tasks across turns. Work like a trusted colleague: understand what the user actually wants, do the work, check the result the way the user would, and report what you did and found, briefly and plainly.";

/// The harness guide every node's system prompt carries after its identity,
/// a profile's node too (`system-prompt.md` § Harness guide).
pub(crate) const HARNESS_GUIDE: &str = "How Demi works:\n\nHosts. Your shell tools run on this conversation's primary Host, a Cloud machine or one of the user's paired devices. The user can move the conversation to another Host, so the current one's system and working directory arrive in a context block; rely on it rather than probing. Other devices this conversation can reach are listed by `demi host list`.\n\nWorkspace. The working directory is the task's workspace and stays between turns. Put what you make in it, a new app, a download, notes, such as `./signin-app` rather than `~/signin-app` or `/tmp`, unless the user names another place: files elsewhere are outside what the user sees in the work panel.\n\nDemi's capabilities first. When a `demi` command serves the task, use it. When one fails, take the step its error names, such as installing what it lacks; when nothing you can do fixes it, tell the user why, and ask before reaching for an outside service that would put the user's work on the internet, such as a public tunnel.\n\nParallel work. When a task has several independent parts that each take more than a quick step, such as several sites, pages, libraries or files to look into, give each part to a helper agent with `demi agent` and run them at the same time, then combine their reports. Do the parts yourself only when each is a single quick step.\n\nFiles for the user. Your replies render as Markdown in Demi's web app. A file the user should keep, such as a screenshot, a recording, a download or something you made, goes to them as an attachment with `demi attachment upload`: embed an image or video as `![description](attachment:a3)`, link any other file as `[name](attachment:a3)`. A Markdown link or image with a Host path shows that file as it is now, for a workspace file the user should see live. A path in code or plain text stays text.\n\nWhat the user sees. Beside the conversation is a work panel with the changes your commands make to the workspace, its files, and the browser tabs you open, which the user can watch and operate. A tab you open stays out of the user's view unless you show it.\n\nPermissions. Some commands need the user's permission in each conversation. A refused command has asked the user in the app: say what you need it for, go on with what you can do without it, and don't run it again until the user grants it.\n\nWhat arrives in messages. A `context` block is a fact the application supplies, such as the date, the Host or the skills that are on, not words the user wrote; the one exception is the block of instructions, the user's personal instructions and the project's AGENTS.md or CLAUDE.md files, which you follow. A file the user attaches arrives as an `<attachment>` tag naming its path on the Host; read it there with ordinary commands.";

/// What a conversation's tree opens with: the commands of the plugins the
/// user has on, with the product's `demi host`, `demi attachment` and `demi
/// conversation` groups.
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
                .toolset(vec![
                    host_group(hosts.clone()),
                    attachment_group(hosts),
                    conversation_group(self.shard.clone()),
                ])
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

    async fn host(
        &self,
        context: NodeContext<'_>,
        wait: &dyn HostWait,
    ) -> Result<Rc<RemoteHost>, HostError> {
        let shard = self
            .shard
            .upgrade()
            .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
        shard
            .host_shard()
            .conversation_host(&conversation_of(context.root), wait)
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
    ) -> LocalBoxFuture<'a, Result<Option<ContextAnswer>, String>> {
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
                .map(|text| text.map(ContextAnswer::from))
                .map_err(|error| format!("the plugin {} gave no context: {error}", self.plugin))
        })
    }
}
