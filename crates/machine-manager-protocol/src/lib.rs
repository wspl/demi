//! The contract between the backend and the Cloud machine manager. The crate
//! holds:
//!
//! - The manager socket's messages and their line codec.
//! - The record of a device's stored images.
//! - The Cloud image manifest.
//! - The pinned gVisor runtime.
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
//!
//! It holds no manager policy and no IO.

pub mod image;
pub mod runtime;
mod state;
mod wire;

/// The format of the manager's state directory's records, which
/// `<data>/format` names (`crates/machine-manager/src/main.rs`). A release
/// that changes a record raises it and migrates the earlier formats.
pub const STATE_FORMAT: u32 = 1;

pub use state::{
    BaseVersion, DeviceId, GenerationId, IdError, MachineImageState, RuntimeState, Volume,
    is_image_name,
};
pub use wire::{
    Capacity, CheckpointParams, CurrentBaseVersionParams, DecodeError, HibernateParams,
    ImageStateParams, MAX_LINE_BYTES, MachineCall, MachineRequest, MachineResponse, Message,
    Operation, ReconcileParams, Reconciled, ResetParams, RuntimeStateParams, VERSION as WIRE_VERSION,
    SandboxMachine, WakeParams, decode_request, decode_response, encode_line,
};
