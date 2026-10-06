//! The Cloud (`managed-hosts.md`): each user's one managed device, a Host
//! behind a runner like a paired device, which the machine manager runs as
//! a gVisor sandbox. The user's shard holds the Cloud's machine: its phase
//! and the capacity permit it holds while it is not stopped, its gate, which
//! every operation on it holds and which an idle stop and a reset reserve,
//! and the transitions that change it: boot and recovery, save,
//! checkpoint and reset. The capacity across users and the machine
//! manager's client are services every shard shares.
//!
//! The Cloud's operations are methods of `dyn CloudShard`, what the Cloud
//! needs of its user's shard (`concurrency.md` § The user shard), which the
//! shard implements.

pub mod access;
pub mod capacity;
pub mod client;
mod growth;
pub mod machine;
mod maintenance;
pub mod reset;
pub mod status;
pub mod tuning;
mod uses;

use std::rc::Rc;
use std::time::Duration;

use demi_backend_database::control::ControlService;
use demi_backend_idle_watch::Activity;
use demi_backend_page_sync::UserMarks;
use demi_backend_providers::llm::assembly::ProviderAssembly;
use demi_backend_providers::vault::entries::Vault;
use demi_backend_runners::devices::Devices;
use demi_backend_runners::public_url::PublicUrl;
use demi_web_api_protocol::ids::{ConversationId, ProviderId, UserId};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::WaitForCancellationFuture;
use tokio_util::task::TaskTracker;

use self::capacity::CloudCapacity;
use self::client::MachinesClient;
use self::machine::Cloud;
use self::tuning::CloudTuning;

/// What every shard's Cloud shares: the machine manager's client, the
/// capacity across users, and the Cloud's settings.
pub struct CloudServices {
    pub machines: MachinesClient,
    pub capacity: CloudCapacity,
    pub tuning: CloudTuning,
}

impl CloudServices {
    pub fn new(machines: MachinesClient, tuning: CloudTuning) -> Self {
        Self {
            capacity: CloudCapacity::new(tuning.capacity),
            machines,
            tuning,
        }
    }
}

/// What the Cloud needs of its user's shard: the handles its operations
/// use, and the user's conversations, which keep the Cloud awake while they
/// work and which an idle stop and a reset hold (`sessions-and-targets.md`
/// § How a conversation uses a device).
pub trait CloudShard {
    fn user(&self) -> &UserId;
    /// The user's Cloud machine.
    fn cloud(&self) -> &Cloud;
    /// The user's devices, the Cloud among them.
    fn devices(&self) -> &Devices;
    fn control(&self) -> &ControlService;
    fn cloud_services(&self) -> &CloudServices;
    /// Where a booting Cloud's runner reaches this backend.
    fn public_url(&self) -> &PublicUrl;
    /// How long a Cloud no conversation uses stays awake
    /// (`resource-lifecycle.md` § Idle window).
    fn idle_window(&self) -> Duration;
    /// The user's pages, which show the Cloud and the devices.
    fn marks(&self) -> UserMarks;
    /// The credential vault, whose entries say which providers run a
    /// process on the Cloud.
    fn vault(&self) -> &Vault;
    fn assembly(&self) -> &ProviderAssembly;
    /// Every task the shard spawns, which its close waits for.
    fn tasks(&self) -> &TaskTracker;
    /// Resolves once the shard starts closing.
    fn closed(&self) -> WaitForCancellationFuture<'_>;
    /// The shard, for a task that outlives the call that starts it.
    fn this(&self) -> Rc<dyn CloudShard>;

    /// What the conversation is doing: a turn of its tree, an operation
    /// holding its file gate, or a user stream someone has open.
    fn activity(&self, conversation: &ConversationId) -> Activity;
    /// The provider entries the live nodes of the conversation's tree infer
    /// with, a subagent's among them; none while its tree is not open.
    fn tree_providers(&self, conversation: &ConversationId) -> Vec<ProviderId>;
    /// Whether someone attends the conversation: a turn of it is in
    /// flight, or a file transfer or user stream of it is open.
    fn attended(&self, conversation: &ConversationId) -> bool;
    /// Holds the conversation for an idle stop, if it does not work: its
    /// tree and its file gate reserved now; none when either is held.
    fn hold_for_idle(&self, conversation: &ConversationId) -> Option<Box<dyn ConversationHold>>;
    /// Holds the conversation for a reset: its turn is interrupted and its
    /// tree held, its file transfers and user streams end when its files
    /// are on the Cloud, and its file gate is reserved once the operations
    /// holding it ended. Each wait has `hold`; none when the conversation
    /// did not let go within it.
    fn hold_for_reset<'a>(
        &'a self,
        conversation: &'a ConversationId,
        files_on_cloud: bool,
        hold: Duration,
    ) -> LocalBoxFuture<'a, Option<Box<dyn ConversationHold>>>;
}

/// What the shard holds of one conversation for an idle stop or a reset of
/// the Cloud; dropping it lets the conversation go.
pub trait ConversationHold {}
