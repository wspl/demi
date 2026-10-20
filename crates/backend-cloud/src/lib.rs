//! The Cloud (`crates/machine-manager/src/lib.rs`): each user's one managed
//! device, a Host behind a runner like a paired device, which the machine
//! manager runs as a gVisor sandbox. The user's shard holds the Cloud's
//! machine: its phase and the capacity permit it holds while it is not
//! stopped, its gate, which every operation on it holds and which an idle
//! stop and a reset reserve, and the transitions that change it: boot and
//! recovery, save, checkpoint and reset. The capacity across users and the
//! machine manager's client are services every shard shares.
//!
//! The Cloud's operations are methods of `dyn CloudShard`, what the Cloud
//! needs of its user's shard, which the shard implements.
//!
//! Device identity survives wake, stop, runtime loss and reset. Concurrent
//! first uses join one allocation and wake, and metadata alone never creates a
//! sandbox. A wake that succeeds means only that the runtime started: the
//! Cloud is ready once its runner has authenticated and the command transport
//! is up, so starting `runsc` alone never marks it ready. A failed boot, save
//! or reset keeps the device and the operation's error, and a retry waits until
//! recovery has established the authoritative state.
//!
//! - A runtime loss while the backend is saving or resetting the device is
//!   ignored, since the operation is already ending the sandbox; one while it
//!   boots counts toward the crash loop, and the boot fails when its runner does
//!   not connect. Crash-loop protection stops repeated automatic boots while a
//!   reset stays possible, and a reset clears the recorded losses.
//! - The idle rule applies across every conversation using the device. Tabs,
//!   resident services, attachments and a look at what runs there do not keep
//!   it active, and maintenance does not restart the idle clock. A running Cloud
//!   hears each conversation's release as a paired device does; a stop needs
//!   none, since it ends everything a release would, and cleanup never wakes a
//!   stopped Cloud.
//! - The lifetime cap stops unattended jobs only after reserving admission and
//!   rechecking active turns and other demand: a turn in flight on a
//!   conversation that uses the Cloud, or a transfer or user stream one of them
//!   has open, postpones it. Otherwise new operations wait behind the
//!   reservation, the jobs nothing attends end with the runner's connection, and
//!   the Cloud stops once their leases are released.
//! - A Cloud that is not stopped holds one of the capacity permits, taken
//!   before its transition starts so that two users cannot both take the last
//!   one; a running Cloud that resets keeps its permit.
//! - While a checkpoint pauses the sandbox the backend suspends its liveness
//!   check of the Cloud's runner, so the pause does not count as a lost
//!   connection. If a Cloud marked running has no connected runner, admission
//!   asks the manager for its runtime state: a stopped runtime starts again over
//!   saved storage, and an existing one gets the runner connection window to
//!   reconnect, without rotating its credential or killing its work; concurrent
//!   requests join that recovery. A lost client socket never establishes that
//!   an operation failed: the backend asks, or reconciles, before it retries.
//!
//! # The Claude Code machine
//!
//! Besides each user's Cloud the manager keeps one more machine, where every
//! CLI process of the Claude Code provider runs, for every user. It holds each
//! account's sign-in in a directory of its own, so the one copy of a sign-in
//! lives where no user has a shell. It is a Cloud in every mechanism here (the
//! same image, a sandbox with its runner, a home kept across stop and wake, the
//! idle rule, checkpoints, the network rules and a capacity permit) and differs
//! in whom it serves: the deployment, not a user. It is in no user's Hosts, no
//! conversation binds it, nobody pairs it, opens a shell in it or resets it, and
//! only the backend's machine access reaches it.
//!
//! Its record belongs to the instance's master, as a managed machine of a
//! second kind beside the master's Cloud (one of each per user at most, the
//! second for the master only), so the master's shard holds it: its runner is
//! adopted there and its phase, admission and idle watch run there. Its
//! activity is the work in flight on it, not any conversation of the master's:
//! each request, login and probe holds a lease for its whole life, an agent's
//! action that infers with Claude Code from its first request to its end, so a
//! turn whose CLI waits fifteen minutes on a shell tool keeps the machine awake
//! although no byte flows. With no lease and no operation for an idle window it
//! stops, and its retained processes are not activity. Another user's work
//! crosses to the master's shard once per process start, never per line: the
//! placement asks the master's shard to start the process and gets back its
//! standard streams as `Send` pipe ends and its end as a `Send` future, which a
//! task on the master's shard relays; dropping them closes the process there,
//! SIGTERM first. Its memory limit is its own setting, since every user's
//! processes share it.

pub mod access;
pub mod capacity;
pub mod client;
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
use demi_web_api_protocol::ids::{ConversationId, DeviceId, ProviderId, UserId};
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

    /// The target the Cloud's executables are built for: the machine
    /// manager runs the image of this machine's architecture, none on an
    /// architecture no image is made for.
    pub fn target(&self) -> Option<&'static str> {
        demi_machine_manager_protocol::image::Architecture::host().map(|architecture| architecture.target())
    }
}

/// What the Cloud needs of its user's shard: the handles its operations
/// use, and the user's conversations, which keep the Cloud awake while they
/// work and which an idle stop and a reset hold (`crates/backend-cloud/src/uses.rs`).
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
    /// (`crates/backend-user-shard/src/lifecycle/mod.rs`).
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
    /// The Cloud `device` stopped, and with it whatever its command
    /// packages kept for its conversations (`crates/command-protocol/src/messages.rs`).
    fn packages_lost(&self, device: &DeviceId);
}

/// What the shard holds of one conversation for an idle stop or a reset of
/// the Cloud; dropping it lets the conversation go.
pub trait ConversationHold {}
