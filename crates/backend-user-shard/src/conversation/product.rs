//! What the backend's conversations are assembled from (`runtime.md`
//! § Sessions and turns): the product's instructions, the command set the
//! user's plugins and the product's `demi host` group make, the Host each
//! node reaches, the conversation's current main Host, and the execution
//! context source, which tells a node that the conversation's execution
//! context changed.

use std::rc::{Rc, Weak};

use demi_agent_tools::{ContextSource, HostResolver, NodeContext};
use demi_backend_host_access::host_commands::host_group;
use demi_backend_host_access::{HostShard, conversation_of};
use demi_backend_remote_host::RemoteHost;
use demi_host_interface::{CommandSet, HostError};
use demi_plugin_interface::EXECUTION_SOURCE;
use demi_shared_types::TurnId;
use futures_util::future::LocalBoxFuture;

use crate::shard::Shard;
use demi_backend_plugins::UserPlugins;

/// The product's instructions, which open every node's system prompt.
pub(crate) const INSTRUCTIONS: &str = "You are a coding agent. Use shell session tools to inspect, edit, test, and verify the workspace.\n\nTreat cwd as the task workspace. Create, edit, and verify task files there by default; do not create a separate project directory under /tmp or another absolute path unless the user asks for it or the workspace is unusable.";

/// The command set of the user's conversations: the commands of the
/// user's plugins and the product's `demi host` group.
pub(crate) fn conversation_commands(plugins: &UserPlugins, shard: Weak<Shard>) -> CommandSet {
    let hosts: Weak<dyn HostShard> = shard;
    plugins
        .commands(vec![host_group(hosts)])
        .expect("the plugins' commands were checked at startup")
}

/// Where a shard's conversations run: each conversation's current main Host.
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
