//! The contract between the backend and the Cloud machine manager
//! (`managed-hosts.md`): the manager socket's messages and their line codec,
//! the record of a device's stored images, and the Cloud image manifest.
//! Both ends link this crate, so each is defined once.

pub mod image;
mod state;
mod wire;

pub use state::{
    BaseVersion, DeviceId, GenerationId, IdError, MachineImageState, RuntimeState, Volume,
    is_image_name,
};
pub use wire::{
    CheckpointParams, CurrentBaseVersionParams, DecodeError, GrowVolumeParams, HibernateParams,
    ImageStateParams, MAX_LINE_BYTES, MachineCall, MachineRequest, MachineResponse, Message,
    Operation, ReconcileParams, ResetParams, RuntimeStateParams, WakeParams, decode_request,
    decode_response, encode_line,
};
