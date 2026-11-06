//! The shard's side of the conversations' host access
//! (`crates/core-host-access/src/access.rs`).
//!
//! It holds what host access needs of the shard (`HostShard`), whose operations
//! the edge and the shard's own work reach through `host_shard`.

use std::rc::Rc;
use std::sync::Arc;

use demi_agent_store::Placement;
use demi_core_blobs::blobs::UserBlobs;
use demi_core_host_access::cloud::CloudHost;
use demi_core_store::store::CoreStore;
use demi_core_store::conversation_index::ConversationChange;
use demi_core_store::conversations::ConversationDb;
use demi_core_store::projects::DirectoryNamed;
use demi_core_host_access::{conversation_of, root_of};
use demi_core_host_access::transition::{ChangeRefusal, Switched};
use demi_core_sync::Part;
use demi_core_host_access::HostShard;
use demi_core_host_access::access::Conversations;
use demi_core_host_access::plugin_files::{DirectorySets, PluginInstalls};
use demi_runner_client::Pipes;
use demi_core_hosts::devices::Devices;
use demi_core_hosts::packages::Packages;
use demi_backend_runners::claims::ClaimRefusal;
use demi_backend_runners::install::install_commands;
use demi_web_api_protocol::devices::{DeviceDto, InstallCommands};
use demi_core_hosts::router::CommandRouter;
use demi_shared_types::Clock;
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use futures_util::future::LocalBoxFuture;
use tokio_util::task::TaskTracker;

use super::Shard;

impl HostShard for Shard {
    fn user(&self) -> &UserId {
        &self.user
    }

    fn store(&self) -> &CoreStore {
        &self.store
    }

    fn clock(&self) -> &Arc<dyn Clock> {
        &self.services.clock
    }

    fn devices(&self) -> &Devices {
        &self.devices
    }

    fn pipes(&self) -> &Pipes {
        &self.pipes
    }

    fn commands(&self) -> &CommandRouter {
        &self.commands
    }

    fn conversations(&self) -> &Conversations {
        &self.conversations
    }

    fn blobs(&self) -> UserBlobs {
        self.services.blobs.for_user(&self.user)
    }

    fn conversation_db(&self, conversation: &ConversationId) -> ConversationDb {
        self.store().databases().db(conversation)
    }

    fn packages(&self) -> &dyn Packages {
        self.services.packages.as_ref()
    }

    fn pair<'a>(&'a self, code: &'a str) -> LocalBoxFuture<'a, Result<DeviceDto, String>> {
        Box::pin(async move {
            match self.services.claims.claim(self.store(), self.user(), code).await {
                Ok(device) => Ok(device),
                Err(ClaimRefusal::UnknownCode) => Err(format!(
                    "{code}: no computer waits under this pairing code; it expired or was never issued. The installer shows a new code when one expires: read the newest one it shows and pass that."
                )),
                Err(ClaimRefusal::TooManyAttempts) => Err(
                    "Too many pairing codes were tried in the last minute; wait a minute, then pass the newest code the installer shows.".into(),
                ),
                Err(ClaimRefusal::Storage(error)) => Err(error.to_string()),
            }
        })
    }

    fn install_commands(&self) -> Option<InstallCommands> {
        let backend = self.services.public_url.get()?;
        Some(install_commands(backend.url()))
    }

    fn tasks(&self) -> &TaskTracker {
        &self.tasks
    }

    fn cloud_host(&self) -> &dyn CloudHost {
        self
    }

    fn this(&self) -> Rc<dyn HostShard> {
        Shard::this(self)
    }

    fn agent_placements(&self, conversation: &ConversationId) -> Vec<Placement> {
        let Some(tree) = self.agent.tree(&root_of(conversation)) else {
            return Vec::new();
        };
        tree.nodes()
            .into_iter()
            .filter_map(|node| node.record().placement)
            .collect()
    }

    fn track_idle(&self, conversation: &ConversationId) {
        Shard::track_idle(self, conversation);
    }

    fn directory_sets(&self) -> LocalBoxFuture<'_, Result<DirectorySets, String>> {
        Box::pin(async move {
            self.plugins
                .directories()
                .await
                .map_err(|error| format!("the plugins' Host directories cannot be read: {error}"))
        })
    }

    fn plugin_installs(&self) -> &PluginInstalls {
        &self.plugin_installs
    }

    fn transition<'a>(
        &'a self,
        conversation: &'a ConversationId,
        change: ConversationChange,
    ) -> LocalBoxFuture<'a, Result<(), ChangeRefusal>> {
        Box::pin(Shard::transition(self, conversation, change))
    }

    fn summary_changed(&self, conversation: &ConversationId) {
        self.mark(Part::Conversation(conversation.clone()));
    }

    fn directory_named(&self, conversation: &ConversationId, named: DirectoryNamed) {
        if named.projects {
            self.mark(Part::Projects);
        }
        for sharer in &named.saved {
            self.mark(Part::Metadata(sharer.clone()));
        }
        let shard = Shard::this(self);
        let conversations = named.conversations;
        let first = conversation.clone();
        self.tasks().spawn_local(async move {
            for conversation in std::iter::once(&first).chain(conversations.iter().filter(|other| **other != first)) {
                shard.mark(Part::Conversation(conversation.clone()));
                // What it kept of its project's files and instructions where
                // its directory was spelled otherwise is let go, and its
                // next request names the directory as it is now.
                shard.project_files().release(conversation);
                shard.agent().placement_changed(&root_of(conversation)).await;
            }
        });
    }

    fn move_conversation<'a>(
        &'a self,
        conversation: &'a ConversationId,
        to: ConversationTarget,
        cancel: &'a tokio_util::sync::CancellationToken,
    ) -> LocalBoxFuture<'a, Result<Option<Switched>, ChangeRefusal>> {
        Box::pin(Shard::move_to(self, conversation, to, cancel))
    }

    fn port_forwards(&self) -> LocalBoxFuture<'_, Result<Vec<demi_web_api_protocol::port_forwards::PortForwardDto>, String>> {
        Box::pin(async move { self.port_forward_list().await.map_err(|error| error.to_string()) })
    }

    fn add_port_forward(
        &self,
        patch: demi_web_api_protocol::port_forwards::PortForwardPatch,
    ) -> LocalBoxFuture<'_, Result<demi_web_api_protocol::port_forwards::PortForwardDto, String>> {
        Box::pin(async move { self.create_port_forward(patch).await.map_err(|error| error.to_string()) })
    }

    fn remove_port_forward(&self, id: demi_web_api_protocol::ids::PortForwardId) -> LocalBoxFuture<'_, Result<(), String>> {
        Box::pin(async move { Shard::remove_port_forward(self, id).await.map_err(|error| error.to_string()) })
    }
}

impl Shard {
    /// The shard as its conversations' host access sees it, whose
    /// operations it is.
    pub fn host_shard(&self) -> &(dyn HostShard + 'static) {
        self
    }
}

impl Shard {
    /// The conversations with a live subagent that works on `device`
    /// (`crates/agent-server/src/node/mod.rs`): a runner's welcome names them
    /// among those the device holds, though their records may name another.
    pub(crate) fn conversations_with_agents_on(&self, device: &DeviceId) -> Vec<ConversationId> {
        self.agent
            .live_roots()
            .into_iter()
            .filter(|root| {
                self.agent.tree(root).is_some_and(|tree| {
                    tree.nodes().iter().any(|node| {
                        node.record()
                            .placement
                            .is_some_and(|placement| placement.device.as_deref() == Some(device.as_str()))
                    })
                })
            })
            .map(|root| conversation_of(&root))
            .collect()
    }
}
