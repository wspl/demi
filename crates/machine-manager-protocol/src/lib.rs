//! The contract between the backend and the Cloud machine manager. The crate
//! holds:
//!
//! - The manager socket's messages and their line codec.
//! - The record of a device's stored images.
//! - The Cloud image manifest.
//! - The pinned gVisor runtime.
//! - Where an integration's shared folder may be mounted.
//!
//! Both ends link this crate, so each is defined once.
//!
//! The backend and the manager of a deployment run one release, so the wire
//! accepts one version. The manager's first message on every connection is
//! `hello`, naming the wire version. A backend of another version closes the
//! connection unused and fails the call that opened it, its start's `reconcile`
//! among them, with an error that names both versions.
//!
//! The wire follows these rules:
//!
//! - A line holds one message, at most 1 MiB. A line that is longer, is not
//!   valid UTF-8 JSON, names an unknown operation or carries malformed parameters
//!   drops its connection, and lines behind it are not served; the manager keeps
//!   serving its other connections. An invalid device id or base version is an
//!   ordinary error reply, and the connection stays usable.
//! - Requests on one connection run concurrently, and replies arrive in
//!   completion order, matched to their requests by id. A reply for a connection
//!   that has closed is discarded and its operation still completes. A death
//!   event goes to every connection.
//! - A line of the manager's log (`MachineResponse::Log`) goes to every open
//!   connection as a death event does, and the backend stores it in its log. A
//!   line made while no connection is open stays in the manager's journal.
//! - A device's status (`ManagerEvent::CloudStatus`) goes to every open
//!   connection when the device's operation ends or its move starts or ends,
//!   and for every device with stored images when a connection opens, so the
//!   backend holds the latest and never asks for it.
//!
//! It holds no manager policy and no IO.

pub mod image;
pub mod isolation;
pub mod network;
pub mod runtime;
pub mod shared;
pub mod store;
mod state;
mod wire;

/// The format of the manager's state directory's records, which
/// `<data>/format` names (`crates/machine-manager/src/main.rs`). A release
/// that changes a record raises it and migrates the earlier formats.
pub const STATE_FORMAT: u32 = 1;

/// Where a stopping manager says how many Clouds it has saved of those it
/// saves, which an upgrade reads while the stop runs: in its runtime
/// directory, which the host sees.
pub const DRAIN_RECORD: &str = "/run/demi-machine-manager/drain.json";

/// What [`DRAIN_RECORD`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DrainRecord {
    pub saved: u32,
    pub saving: u32,
}

/// The manager's systemd unit, which an upgrade stops and starts, and which
/// a manager restarts to run the gVisor release it fetched.
pub const MANAGER_UNIT: &str = "demi-machine-manager.service";

/// What an upgrade writes before it stops the manager when the release it
/// moves to keeps [`STATE_FORMAT`]: the manager that stops then leaves its
/// running sandboxes to the next one, which takes them over, rather than
/// saving them (`crates/machine-manager/src/handover.rs`). A manager that
/// knows no such request saves them, as before.
pub const LEAVE_REQUEST: &str = "/run/demi-machine-manager/leave";

/// Where a manager that left its sandboxes records them for the next one,
/// in its runtime directory, which a reboot empties as it ends them.
pub const LEFT_RECORD: &str = "/run/demi-machine-manager/left.json";

/// What [`LEFT_RECORD`] holds: each sandbox the manager left running, gVisor
/// sandboxes and Simple Isolation Clouds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LeftRecord {
    pub sandboxes: Vec<LeftSandbox>,
    /// Absent from a record of a release without Simple Isolation.
    #[serde(default)]
    pub simple: Vec<LeftSimple>,
}

/// A Simple Isolation Cloud a manager left running: its device, and its
/// proot's process, which the next manager takes over only while the
/// device's own record names the same one
/// (`crates/machine-manager/src/runtime/simple.rs`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LeftSimple {
    pub device: DeviceId,
    pub pid: i32,
    pub started: u64,
}

/// A sandbox a manager left running: its device, the boot its working pair
/// records, the base whose `/opt/demi` it mounts and the shared folder it
/// mounts, which the next manager keeps while the sandbox runs, and which
/// machine it is, which decides whether the network policy applies to it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LeftSandbox {
    pub device: DeviceId,
    pub id: String,
    pub slot: u16,
    pub programs: BaseVersion,
    /// Absent from a record an earlier release wrote, which does not say
    /// whether its sandbox is the Claude Code machine: it is then taken as
    /// one, which reaches the internet alone whatever the network policy,
    /// until its next boot.
    #[serde(default = "unknown_machine")]
    pub machine: SandboxMachine,
    /// The id of the integration's shared folder it mounts; none for none,
    /// and in a record of a manager before shared folders.
    #[serde(default)]
    pub shared: Option<String>,
}

fn unknown_machine() -> SandboxMachine {
    SandboxMachine::ClaudeCode
}

pub use state::{
    BaseVersion, DeviceId, GenerationId, IdError, MachineImageState, RuntimeState, Volume,
    is_image_name,
};
pub use isolation::{Download, Enforces, Isolation, MoveStatus, Offer, RuntimeOffer, Unavailable};
pub use network::{Allowed, Destination, Domain, DomainName, NetworkRules};
pub use wire::{
    Capacity, CheckpointParams, CurrentBaseVersionParams, DecodeError, DeleteParams, DeleteSharedParams, HibernateParams,
    ImageStateParams, MAX_LINE_BYTES, MachineCall, MachineRequest, MachineResponse, ManagerEvent, Message,
    Operation, ReconcileParams, Reconciled, ResetParams, RuntimeStateParams, VERSION as WIRE_VERSION,
    SandboxMachine, SetIsolationParams, SetNetworkRulesParams, SharedFolder, WakeParams, decode_request,
    decode_response, encode_line,
};

/// The room a large write to a server's disk leaves free: the backend's
/// databases and logs keep writing while it runs, and a full disk breaks them,
/// as the search index and the system log broke on a server whose disk a
/// checkpoint and an update's preparation filled together. The machine
/// manager's writes to its store and `demi-server`'s fetch of a release check
/// for it.
pub const ROOM_RESERVE: u64 = 2 << 30;
