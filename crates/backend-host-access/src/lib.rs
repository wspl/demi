//! The conversation's host access: the one way to a conversation's primary or
//! attached Host, with target resolution and the transitions that end a
//! target (switch, archive, detach); file transfers, uploads, remote files,
//! user streams and the file watch; the shell environments of the agent's
//! nodes over it; and the product's `demi host` and `demi attachment` groups.
//!
//! Host access's operations are methods of `dyn HostShard`, what host access
//! needs of its user's shard (`crates/backend-user-shard/src/lib.rs`), which the
//! shard implements.
//!
//! Everything that reaches a conversation's Host on the conversation's behalf
//! goes through it, whether the agent resolves its Host for a tool call or
//! the backend writes an attachment, lists the working tree or reads a file
//! for the web app. For every kind of target it resolves the target, refuses
//! an archived conversation, wakes a stopped Cloud and holds it for the
//! operation, and takes the conversation's file gate so the operation
//! excludes an archive or a target switch. A plugin never writes to a Host:
//! the Host directories its user needs are installed by a job's own
//! admission, and its reads of a conversation's files never wake the Host.
//! Any other change to wake or gate behavior is decided here, not introduced
//! as a bypass.
//!
//! These are every way to a Host; the last three touch no conversation's files:
//!
//! ```text
//! way                    takes                       a stopped Cloud
//! ---------------------  --------------------------  -------------------
//! conversation access    file gate, Cloud admission  woken; reset waited
//! stream admission       file gate while admitting   not woken
//! lifecycle access       nothing (a runner message)  not woken
//! device access          nothing; caller owns it     not woken
//! machine access         the machine's admission     woken; reset waited
//! ```
//!
//! Stream admission serves user streams, one-shot user calls that must not
//! wake the Host and a plugin's reads of a conversation's files; lifecycle
//! access is the conversation release, which a stopped Cloud hears after its
//! next wake. Device access is the owner's work on a device whose runner is
//! connected (a direct channel, the device log, browsing directories to
//! choose a target, a port forward's listener): it takes no file gate and
//! checks no conversation's binding, so a transition does not wait for it, and
//! it never wakes a Cloud, since a log is read when its Host runs again and
//! showing a version is no reason to wake one. Machine access is for work that
//! needs the machine itself, such as creating a Cloud project's directory or a
//! provider's process on the Claude Code machine.
//!
//! The state this protects lives in the user's shard because one shard holds
//! all of a user's conversations, devices and Cloud: the file gates, the
//! admission of transfers and streams, and the transitions that end them. The
//! edge moves the bytes of transfers and user streams; admitting them is the
//! shard's. A runner connection makes a conversation's Host handle only
//! against a lease of that conversation's file gate, which only host access
//! takes, so no other code can make one.
//!
//! Other work holds the conversation through the gate itself (`file_gate`),
//! whose leases make no Host. Host access never sees `Shard`, and no second way
//! to a conversation's Host exists.

pub mod access;
pub mod attachment_commands;
pub mod directories;
pub mod expose_commands;
pub mod file_watch;
pub mod forward_commands;
pub mod host_commands;
pub mod kept;
pub mod lease;
pub mod plugin_files;
pub mod remote_files;
pub mod shells;
pub mod stream;
pub mod target;
pub mod transfer;
pub mod transition;
pub mod uploads;

use std::rc::Rc;
use std::sync::Arc;

use demi_backend_blobs::blobs::UserBlobs;
use demi_backend_cloud::CloudShard;
use demi_backend_database::control::ControlService;
use demi_backend_database::conversations::ConversationDb;
use demi_backend_remote_host::Pipes;
use demi_backend_runners::claims::PendingClaims;
use demi_backend_runners::devices::Devices;
use demi_backend_runners::native::NativeCatalog;
use demi_backend_runners::public_url::PublicUrl;
use demi_backend_runners::router::CommandRouter;
use demi_backend_database::conversation_index::ConversationChange;
use demi_shared_types::{Clock, NodeId};
use demi_web_api_protocol::ids::{ConversationId, UserId};
use tokio_util::task::TaskTracker;

use futures_util::future::LocalBoxFuture;

use self::access::Conversations;
use self::plugin_files::{DirectorySets, PluginInstalls};
use self::transition::ChangeRefusal;

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
    /// The command packages the conversations' commands bind to.
    fn native(&self) -> &NativeCatalog;
    /// Where runners fetch the packages' executables from.
    fn public_url(&self) -> &PublicUrl;
    /// The runners waiting to be paired, which `demi host pair` claims.
    fn claims(&self) -> &PendingClaims;
    /// Every task the shard spawns, which its close waits for.
    fn tasks(&self) -> &TaskTracker;
    /// The shard as the user's Cloud sees it, whose admission an operation
    /// on the Cloud takes.
    fn cloud_shard(&self) -> &(dyn CloudShard + 'static);
    /// The shard, for a task that outlives the call that starts it.
    fn this(&self) -> Rc<dyn HostShard>;
    /// Starts the conversation's idle watch unless one runs, as each Host
    /// admission does (`crates/backend-user-shard/src/lifecycle/mod.rs`).
    fn track_idle(&self, conversation: &ConversationId);
    /// The Host directories of the user's plugins, which each job installs
    /// before it starts.
    fn directory_sets(&self) -> LocalBoxFuture<'_, Result<DirectorySets, String>>;
    /// What the shard remembers of its Hosts' directories.
    fn plugin_installs(&self) -> &PluginInstalls;
    /// Applies `change` to the conversation through the shard's one entry
    /// for its changes, and shows it on the user's pages, as a change the
    /// user makes.
    fn transition<'a>(
        &'a self,
        conversation: &'a ConversationId,
        change: ConversationChange,
    ) -> LocalBoxFuture<'a, Result<(), ChangeRefusal>>;
    /// Sends the conversation's summary to the user's pages again, after a
    /// change of a fact it carries that no `transition` makes, such as a new
    /// attachment.
    fn summary_changed(&self, conversation: &ConversationId);
    /// Makes the conversation's pending changes, a move or a detach an agent
    /// asked for, once its tree is next idle (`crates/backend-host-access/src/transition.rs`).
    fn settle_when_idle(&self, conversation: &ConversationId);
    /// The user's port forwards, as `demi host forward list` names them.
    fn port_forwards(&self) -> LocalBoxFuture<'_, Result<Vec<demi_web_api_protocol::port_forwards::PortForwardDto>, String>>;
    /// Adds a port forward from its defaults with `patch` over them.
    fn add_port_forward(
        &self,
        patch: demi_web_api_protocol::port_forwards::PortForwardPatch,
    ) -> LocalBoxFuture<'_, Result<demi_web_api_protocol::port_forwards::PortForwardDto, String>>;
    /// Removes the port forward `id`.
    fn remove_port_forward(
        &self,
        id: demi_web_api_protocol::ids::PortForwardId,
    ) -> LocalBoxFuture<'_, Result<(), String>>;
    /// The user's exposed ports, as `demi host expose list` names them.
    fn exposed_ports(
        &self,
    ) -> LocalBoxFuture<'_, Result<Vec<demi_web_api_protocol::exposed_ports::ExposedPortDto>, String>>;
    /// Exposes a port of one of the user's devices.
    fn add_exposed_port(
        &self,
        new: demi_web_api_protocol::exposed_ports::NewExposedPort,
    ) -> LocalBoxFuture<'_, Result<demi_web_api_protocol::exposed_ports::ExposedPortDto, String>>;
    /// Stops exposing the exposed port `id`.
    fn remove_exposed_port(
        &self,
        id: demi_web_api_protocol::ids::ExposedPortId,
    ) -> LocalBoxFuture<'_, Result<(), String>>;
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
