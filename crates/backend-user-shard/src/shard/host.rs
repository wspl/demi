//! The shard's side of the conversations' host access
//! (`sessions-and-targets.md` § Host operations): what host access needs of
//! the shard (`HostShard`), whose operations the edge and the shard's own
//! work reach through `host_shard`.

use std::rc::Rc;
use std::sync::Arc;

use demi_backend_blobs::blobs::UserBlobs;
use demi_backend_cloud::CloudShard;
use demi_backend_database::control::ControlService;
use demi_backend_database::conversations::ConversationDb;
use demi_backend_host_access::HostShard;
use demi_backend_host_access::access::Conversations;
use demi_backend_host_access::plugin_files::{DirectorySets, PluginInstalls};
use demi_backend_page_sync::Part;
use demi_backend_remote_host::Pipes;
use demi_backend_runners::devices::Devices;
use demi_backend_runners::native::NativeCatalog;
use demi_backend_runners::public_url::PublicUrl;
use demi_backend_runners::router::CommandRouter;
use demi_plugin_interface::Topic;
use demi_shared_types::Clock;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use futures_util::future::LocalBoxFuture;
use tokio_util::task::TaskTracker;

use super::Shard;

impl HostShard for Shard {
    fn user(&self) -> &UserId {
        &self.user
    }

    fn control(&self) -> &ControlService {
        &self.services.control
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
        self.services.conversations.db(conversation)
    }

    fn native(&self) -> &NativeCatalog {
        &self.services.native
    }

    fn public_url(&self) -> &PublicUrl {
        &self.services.public_url
    }

    fn tasks(&self) -> &TaskTracker {
        &self.tasks
    }

    fn cloud_shard(&self) -> &(dyn CloudShard + 'static) {
        Shard::cloud_shard(self)
    }

    fn this(&self) -> Rc<dyn HostShard> {
        Shard::this(self)
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

    fn job_ended(&self, conversation: &ConversationId) {
        *self
            .jobs_ended
            .borrow_mut()
            .entry(conversation.clone())
            .or_default() += 1;
        self.plugins.fire(Topic::Jobs, Some(conversation));
        self.services
            .sync
            .mark(&self.user, Part::Conversation(conversation.clone()));
    }
}

impl Shard {
    /// How many of the conversation's jobs ended since the shard started,
    /// its summary's working-tree revision.
    pub(crate) fn working_tree_revision(&self, conversation: &ConversationId) -> u64 {
        self.jobs_ended
            .borrow()
            .get(conversation)
            .copied()
            .unwrap_or(0)
    }

    /// The shard as its conversations' host access sees it, whose
    /// operations it is.
    pub fn host_shard(&self) -> &(dyn HostShard + 'static) {
        self
    }
}
