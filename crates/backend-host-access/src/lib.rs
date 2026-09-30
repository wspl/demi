//! The conversation's host access (`sessions-and-targets.md` § Host
//! operations): the one way to a conversation's main or attached Host, with
//! target resolution and the transitions that end a target (switch, archive,
//! detach); file transfers, uploads, remote files and user streams; the
//! shell environments of the agent's nodes over it; and the product's
//! `demi host` group with its `expose` leaves.
//!
//! Host access's operations are methods of `dyn HostShard`, what host access
//! needs of its user's shard (`concurrency.md` § The user shard), which the
//! shard implements.

pub mod access;
pub mod blobs;
pub mod expose_commands;
pub mod host_commands;
pub mod lease;
pub mod remote_files;
pub mod shells;
pub mod stream;
pub mod target;
pub mod transfer;
pub mod transition;
pub mod uploads;

use std::rc::Rc;
use std::sync::Arc;

use demi_backend_cloud::CloudShard;
use demi_backend_expose::ExposeShard;
use demi_backend_objects::blobs::UserBlobs;
use demi_backend_runners::devices::Devices;
use demi_backend_runners::native::NativeCatalog;
use demi_backend_runners::public_url::PublicUrl;
use demi_backend_runners::router::CommandRouter;
use demi_backend_storage::control::ControlService;
use demi_backend_storage::conversations::ConversationDb;
use demi_core::{Clock, NodeId};
use demi_host_remote::Pipes;
use demi_web_api::ids::{ConversationId, UserId};
use tokio_util::task::TaskTracker;

use self::access::Conversations;

/// What host access needs of its user's shard: the handles its operations
/// use, and the conversation's idle watch, which every Host admission starts.
pub trait HostShard {
    fn user(&self) -> &UserId;
    fn control(&self) -> &ControlService;
    /// The wall clock the backend reads times from.
    fn clock(&self) -> &Arc<dyn Clock>;
    /// The user's devices, each with its runner connection.
    fn devices(&self) -> &Devices;
    /// The pipes of the user's devices.
    fn pipes(&self) -> &Pipes;
    /// Each agent node's commands, for the rpc calls of its jobs.
    fn commands(&self) -> &CommandRouter;
    /// Each conversation's slot, with its file gate and transfers.
    fn conversations(&self) -> &Conversations;
    /// The user's blob namespace.
    fn blobs(&self) -> UserBlobs;
    /// The database of the user's conversation `conversation`.
    fn conversation_db(&self, conversation: &ConversationId) -> ConversationDb;
    /// The native packages the conversations' commands bind to.
    fn native(&self) -> &NativeCatalog;
    /// Where runners fetch the packages' executables from.
    fn public_url(&self) -> &PublicUrl;
    /// Every task the shard spawns, which its close waits for.
    fn tasks(&self) -> &TaskTracker;
    /// The shard as the user's Cloud sees it, whose admission an operation
    /// on the Cloud takes.
    fn cloud_shard(&self) -> &(dyn CloudShard + 'static);
    /// The shard as the user's exposes see it, which `demi host expose`
    /// acts on.
    fn expose_shard(&self) -> &(dyn ExposeShard + 'static);
    /// The shard, for a task that outlives the call that starts it.
    fn this(&self) -> Rc<dyn HostShard>;
    /// Starts the conversation's idle watch unless one runs, as each Host
    /// admission does (`resource-lifecycle.md` § Idle window).
    fn track_idle(&self, conversation: &ConversationId);
}

/// The root node of a conversation's tree, whose id is the conversation's in
/// the spelling the index keeps.
pub fn root_of(conversation: &ConversationId) -> NodeId {
    NodeId::try_from(conversation.as_str()).expect("a conversation id is never empty")
}

/// The conversation a tree's root belongs to. The shard opens trees only
/// under the roots `root_of` names, so every root is a conversation id.
pub fn conversation_of(root: &NodeId) -> ConversationId {
    ConversationId::try_from(root.as_str()).expect("the shard opens trees only for conversations")
}
