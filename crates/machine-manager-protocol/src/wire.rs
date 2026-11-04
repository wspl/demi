//! The socket's messages: one JSON document per line, a request from the
//! backend, the manager's reply to it, and the manager's unsolicited events. Fields are declared in the order a message is written.

use std::num::NonZeroU64;

use demi_runner_protocol::boot::ManagedBoot;
use demi_shared_types::{CloudLimit, LogLine};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{BaseVersion, Isolation, MachineImageState, MoveStatus, NetworkRules, RuntimeOffer, RuntimeState, Volume};

/// The longest line either end reads, its newline excluded: a reader holds a
/// line whole, so an end that never sends a newline would fill memory. The
/// largest real message is a few kilobytes.
pub const MAX_LINE_BYTES: usize = 1 << 20;

/// The wire's version, which the manager's `hello` names. The backend and
/// the manager of a deployment run one release, so a change to any message
/// changes it.
pub const VERSION: u32 = 11;

/// A request: an id the client chooses, which its reply names, and the call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
pub struct MachineRequest {
    #[garde(length(min = 1))]
    pub id: String,
    #[serde(flatten)]
    #[garde(dive)]
    pub call: MachineCall,
}

/// An operation and its parameters. Keys a message does not declare are
/// ignored. A device id or base version travels as a string: a name that
/// breaks the image-name rule is the operation's error, not a malformed
/// message, so the connection stays usable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(tag = "op", content = "params", rename_all = "snake_case")]
pub enum MachineCall {
    Reconcile(#[garde(skip)] ReconcileParams),
    CurrentBaseVersion(#[garde(skip)] CurrentBaseVersionParams),
    ImageState(#[garde(dive)] ImageStateParams),
    RuntimeState(#[garde(dive)] RuntimeStateParams),
    Wake(#[garde(dive)] WakeParams),
    Hibernate(#[garde(dive)] HibernateParams),
    Checkpoint(#[garde(dive)] CheckpointParams),
    Reset(#[garde(dive)] ResetParams),
    SetNetworkRules(#[garde(skip)] SetNetworkRulesParams),
    SetIsolation(#[garde(skip)] SetIsolationParams),
    Delete(#[garde(dive)] DeleteParams),
    DeleteShared(#[garde(dive)] DeleteSharedParams),
}

impl MachineCall {
    /// The operation's name on the wire, such as `image_state`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reconcile(_) => "reconcile",
            Self::CurrentBaseVersion(_) => "current_base_version",
            Self::ImageState(_) => "image_state",
            Self::RuntimeState(_) => "runtime_state",
            Self::Wake(_) => "wake",
            Self::Hibernate(_) => "hibernate",
            Self::Checkpoint(_) => "checkpoint",
            Self::Reset(_) => "reset",
            Self::SetNetworkRules(_) => "set_network_rules",
            Self::SetIsolation(_) => "set_isolation",
            Self::Delete(_) => "delete",
            Self::DeleteShared(_) => "delete_shared",
        }
    }
}

/// Report the devices the manager runs, which stay running, recover
/// incomplete operations, and install the network policy again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileParams {}

/// Replace what Clouds reach, which the firewall and the resolver apply at
/// once. The manager keeps the rules in memory only and starts with the
/// [default](NetworkRules::default), so the backend sends them each time it
/// connects as well as each time the master changes the network policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetNetworkRulesParams {
    pub network_rules: NetworkRules,
}

/// Choose the isolation new devices start on and every device moves to once
/// it is stopped. The manager keeps it in memory only, and until the backend
/// names one runs new devices on gVisor where it can and on Simple
/// Isolation elsewhere, so the backend sends it each time it connects as
/// well as each time the master changes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetIsolationParams {
    pub isolation: Isolation,
}

/// What a `reconcile` found: the devices whose sandboxes run, by id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reconciled {
    pub running: Vec<String>,
}

/// Read the configured base.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentBaseVersionParams {}

/// Read a device's committed generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct ImageStateParams {
    #[garde(length(min = 1))]
    pub device_id: String,
}

/// Read whether the manager runs a sandbox for a device, after the device's
/// earlier operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStateParams {
    #[garde(length(min = 1))]
    pub device_id: String,
}

/// The capacities, in bytes, of a device's system and home filesystems,
/// which the backend names from the account's setting
/// (`crates/backend-cloud/src/lib.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capacity {
    pub system_bytes: NonZeroU64,
    pub home_bytes: NonZeroU64,
}

impl Capacity {
    /// The capacity of `volume`.
    pub fn bytes(&self, volume: Volume) -> NonZeroU64 {
        match volume {
            Volume::System => self.system_bytes,
            Volume::Home => self.home_bytes,
        }
    }
}

/// Create first-use storage or recover existing storage at `capacity`,
/// enlarging a smaller image, then start one sandbox with the boot
/// credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct WakeParams {
    #[garde(length(min = 1))]
    pub device_id: String,
    #[garde(skip)]
    pub capacity: Capacity,
    /// Checked as it is read: an unknown key refuses the message.
    #[garde(skip)]
    pub boot: ManagedBoot,
    /// Which machine the device is, whose memory limit the sandbox gets.
    #[garde(skip)]
    pub machine: SandboxMachine,
    /// The shared folder the sandbox mounts, made at `capacity` on its
    /// first use; none for a device of no integration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub shared: Option<SharedFolder>,
    /// The sandbox's CPU budget and memory limit with the resource limits
    /// on, which an integration sizes: a number, or none at all; the
    /// manager's own settings for a field that is none. A limit beyond the
    /// host's CPUs or memory refuses the wake.
    #[serde(default, skip_serializing_if = "SandboxLimits::is_empty")]
    #[garde(skip)]
    pub limits: SandboxLimits,
}

/// A sandbox's CPU budget, in CPUs, and memory limit, in MiB, or none at
/// all; a field that is none for the manager's setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<CloudLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mib: Option<CloudLimit>,
}

impl SandboxLimits {
    pub fn is_empty(&self) -> bool {
        self.cpus.is_none() && self.memory_mib.is_none()
    }
}

/// An integration's shared folder (`crate::shared`): an image of its own,
/// which every sandbox that names it mounts, writable, at `path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolder {
    /// The image's name, the integration's id.
    #[garde(length(min = 1))]
    pub id: String,
    #[garde(skip)]
    pub capacity_bytes: NonZeroU64,
    /// The absolute path in the sandbox, which [`crate::shared::mount_path`]
    /// answered.
    #[garde(length(min = 2))]
    pub path: String,
}

/// Which managed machine a sandbox runs
/// (`crates/backend-cloud/src/lib.rs`): a user's Cloud, or the Claude
/// Code machine, which has a memory limit of its own and keeps only its
/// current generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMachine {
    Cloud,
    ClaudeCode,
}

/// Stop execution, save storage and release runtime resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct HibernateParams {
    #[garde(length(min = 1))]
    pub device_id: String,
    /// Which machine the device is, whose generations the save keeps.
    #[garde(skip)]
    pub machine: SandboxMachine,
}

/// Publish the running device's storage while preserving its processes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointParams {
    #[garde(length(min = 1))]
    pub device_id: String,
    /// Which machine the device is, whose generations the save keeps.
    #[garde(skip)]
    pub machine: SandboxMachine,
}

/// Publish a clean system on `base_version` at `capacity`'s system bytes
/// with the retained home, a first home at its home bytes on a device that
/// has none, once per `operation_id`; does not boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct ResetParams {
    #[garde(length(min = 1))]
    pub device_id: String,
    #[garde(length(min = 1))]
    pub operation_id: String,
    #[garde(length(min = 1))]
    pub base_version: String,
    #[garde(skip)]
    pub capacity: Capacity,
    /// Which machine the device is, whose generations the save keeps.
    #[garde(skip)]
    pub machine: SandboxMachine,
}

/// Stop a device's sandbox and remove all its storage, once per device:
/// a device that has none answers at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct DeleteParams {
    #[garde(length(min = 1))]
    pub device_id: String,
}

/// Remove a shared folder's image; refused while a sandbox mounts it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSharedParams {
    #[garde(length(min = 1))]
    pub id: String,
}

/// A message from the manager: the reply to a request, or the death of a
/// device's sandbox or a line of its log, which every connection receives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum MachineResponse {
    /// The manager's first message on a connection: the wire version it
    /// speaks, and what it can do with each isolation.
    Hello {
        #[garde(skip)]
        version: u32,
        #[garde(skip)]
        runtimes: Vec<RuntimeOffer>,
    },
    /// The operation's result, which the client decodes as the operation's
    /// [`Operation::Output`] once it knows which request this answers.
    Ok {
        #[garde(length(min = 1))]
        id: String,
        #[garde(skip)]
        result: serde_json::Value,
    },
    Error {
        #[garde(length(min = 1))]
        id: String,
        #[garde(skip)]
        message: String,
    },
    /// A device's sandbox exited without being asked to stop.
    Death {
        #[garde(length(min = 1))]
        device_id: String,
    },
    /// What changed in the manager that the backend shows, sent on every open
    /// connection as a death is, and for every device when a connection opens.
    Event {
        #[garde(skip)]
        event: ManagerEvent,
    },
    /// A line of the manager's log, sent on every open connection as a death
    /// is: the backend stores it in its log. A line made while no connection
    /// is open is not sent.
    Log {
        #[garde(skip)]
        line: LogLine,
    },
}

/// What the manager tells the backend unasked, so the backend never asks it
/// for what a page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ManagerEvent {
    /// A device's stored image, or the base the manager is configured with,
    /// is as this says: the capacities of its two filesystems, none on
    /// Simple Isolation, whose files have none, and before its first
    /// generation; whether its system is on another base than the configured
    /// one; the isolation its storage is on, and its move to the other.
    CloudStatus {
        device_id: String,
        volumes: Option<Capacity>,
        newer_image: bool,
        isolation: Isolation,
        move_status: Option<MoveStatus>,
    },
    /// What the manager can do with each isolation changed since its hello,
    /// as a download of a runtime's release changes it.
    Runtimes { runtimes: Vec<RuntimeOffer> },
    /// The host's CPUs and memory, which a sandbox without a limit may use,
    /// and the limits of a Cloud whose wake names none; none with the
    /// limits off. Sent once when a connection opens.
    ServerResources {
        host: Resources,
        cloud: Option<Resources>,
    },
}

/// CPUs and memory, of a host or of one sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resources {
    pub cpus: u32,
    pub memory_mib: u64,
}

/// A call's parameters, and the result its `ok` reply carries.
pub trait Operation: Into<MachineCall> {
    type Output: Serialize + DeserializeOwned;
}

macro_rules! operations {
    ($($params:ident => $variant:ident: $output:ty,)+) => {
        $(
            impl From<$params> for MachineCall {
                fn from(params: $params) -> Self {
                    Self::$variant(params)
                }
            }

            impl Operation for $params {
                type Output = $output;
            }
        )+
    };
}

operations! {
    ReconcileParams => Reconcile: Reconciled,
    CurrentBaseVersionParams => CurrentBaseVersion: BaseVersion,
    ImageStateParams => ImageState: Option<MachineImageState>,
    RuntimeStateParams => RuntimeState: RuntimeState,
    WakeParams => Wake: (),
    HibernateParams => Hibernate: (),
    CheckpointParams => Checkpoint: (),
    ResetParams => Reset: (),
    SetNetworkRulesParams => SetNetworkRules: (),
    SetIsolationParams => SetIsolation: (),
    DeleteParams => Delete: (),
    DeleteSharedParams => DeleteShared: (),
}

/// A line that is not a message of this wire.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error(transparent)]
    Shape(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

impl From<garde::Report> for DecodeError {
    fn from(report: garde::Report) -> Self {
        Self::Invalid(report.to_string().trim_end().to_owned())
    }
}

/// Decodes one request line, its newline removed.
pub fn decode_request(line: &str) -> Result<MachineRequest, DecodeError> {
    decode(line)
}

/// Decodes one response line, its newline removed.
pub fn decode_response(line: &str) -> Result<MachineResponse, DecodeError> {
    decode(line)
}

fn decode<M: DeserializeOwned + garde::Validate<Context = ()>>(
    line: &str,
) -> Result<M, DecodeError> {
    let message: M = serde_json::from_str(line)?;
    message.validate()?;
    Ok(message)
}

/// A message of this wire.
pub trait Message: Serialize + sealed::Sealed {}

impl Message for MachineRequest {}
impl Message for MachineResponse {}

mod sealed {
    pub trait Sealed {}

    impl Sealed for super::MachineRequest {}
    impl Sealed for super::MachineResponse {}
}

/// The line that carries `message`: compact JSON and a newline.
pub fn encode_line(message: &impl Message) -> Vec<u8> {
    // Serializing fails only for a map with keys that are not strings or a
    // value whose `Serialize` fails; no message of this wire holds either.
    let mut line = serde_json::to_vec(message).expect("a machine message encodes as JSON");
    line.push(b'\n');
    line
}
