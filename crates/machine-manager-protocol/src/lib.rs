//! The contract between the backend and the Cloud machine manager
//! (`managed-hosts.md`): the manager socket's messages and their line codec,
//! the record of a device's stored images, the Cloud image manifest, and the
//! pinned gVisor runtime. Both ends link this crate, so each is defined once.

pub mod image;
pub mod runtime;
mod state;
mod wire;

/// The format of the manager's state directory's records, which
/// `<data>/format` names (`managed-hosts.md` § Startup and recovery). A
/// release that changes a record raises it and migrates the earlier formats.
pub const STATE_FORMAT: u32 = 1;

pub use state::{
    BaseVersion, DeviceId, GenerationId, IdError, MachineImageState, RuntimeState, Volume,
    is_image_name,
};
pub use wire::{
    CheckpointParams, CurrentBaseVersionParams, DecodeError, GrowVolumeParams, HibernateParams,
    ImageStateParams, MAX_LINE_BYTES, MachineCall, MachineRequest, MachineResponse, Message,
    Operation, ReconcileParams, ResetParams, RuntimeStateParams, VERSION as WIRE_VERSION,
    WakeParams, decode_request, decode_response, encode_line,
};
