//! The socket's messages (`managed-hosts.md` § Control and ownership): one
//! JSON document per line, a request from the backend, the manager's reply
//! to it, and the manager's unsolicited death event. Fields are declared in
//! the order a message is written.

use std::num::NonZeroU64;

use demi_runner_protocol::boot::ManagedBoot;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{BaseVersion, MachineImageState, RuntimeState, Volume};

/// The longest line either end reads, its newline excluded. The largest real
/// message is a few kilobytes.
pub const MAX_LINE_BYTES: usize = 1 << 20;

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
    GrowVolume(#[garde(dive)] GrowVolumeParams),
    Reset(#[garde(dive)] ResetParams),
}

impl MachineCall {
    /// The operation's name on the wire, such as `grow_volume`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reconcile(_) => "reconcile",
            Self::CurrentBaseVersion(_) => "current_base_version",
            Self::ImageState(_) => "image_state",
            Self::RuntimeState(_) => "runtime_state",
            Self::Wake(_) => "wake",
            Self::Hibernate(_) => "hibernate",
            Self::Checkpoint(_) => "checkpoint",
            Self::GrowVolume(_) => "grow_volume",
            Self::Reset(_) => "reset",
        }
    }
}

/// Stop and save every device, recover incomplete operations, and install
/// the network policy again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileParams {}

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

/// Create first-use storage or recover existing storage, then start one
/// sandbox with the boot credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct WakeParams {
    #[garde(length(min = 1))]
    pub device_id: String,
    /// Checked as it is read: an unknown key refuses the message.
    #[garde(skip)]
    pub boot: ManagedBoot,
}

/// Stop execution, save storage and release runtime resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct HibernateParams {
    #[garde(length(min = 1))]
    pub device_id: String,
}

/// Publish the running device's storage while preserving its processes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointParams {
    #[garde(length(min = 1))]
    pub device_id: String,
}

/// Grow one of the running device's filesystems to at least `bytes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct GrowVolumeParams {
    #[garde(length(min = 1))]
    pub device_id: String,
    #[garde(skip)]
    pub volume: Volume,
    #[garde(skip)]
    pub bytes: NonZeroU64,
}

/// Publish a clean system on `base_version` with the retained home, once
/// per `operation_id`; does not boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct ResetParams {
    #[garde(length(min = 1))]
    pub device_id: String,
    #[garde(length(min = 1))]
    pub operation_id: String,
    #[garde(length(min = 1))]
    pub base_version: String,
}

/// A message from the manager: the reply to a request, or the death of a
/// device's sandbox, which every connection receives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum MachineResponse {
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
    ReconcileParams => Reconcile: (),
    CurrentBaseVersionParams => CurrentBaseVersion: BaseVersion,
    ImageStateParams => ImageState: Option<MachineImageState>,
    RuntimeStateParams => RuntimeState: RuntimeState,
    WakeParams => Wake: (),
    HibernateParams => Hibernate: (),
    CheckpointParams => Checkpoint: (),
    GrowVolumeParams => GrowVolume: (),
    ResetParams => Reset: (),
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

fn decode<M: DeserializeOwned + garde::Validate<Context = ()>>(line: &str) -> Result<M, DecodeError> {
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
