//! A device's working pair (`managed-hosts.md` § Images): the private
//! writable copies of its committed images, with a manifest that names the
//! generation they came from and their current capacities.
//!
//! ```text
//! <data>/working/<device>/manifest.json   the working pair's record
//! <data>/working/<device>/system.ext4, home.ext4
//! <data>/working/<device>/sandbox.json    while a sandbox runs on it
//! ```

use std::{
    io,
    path::{Path, PathBuf},
};

use demi_machines_protocol::{DeviceId, GenerationId, MachineImageState};

use super::{
    durable::{remove_tree, sync, write_json},
    ext4::{self, Ext4Error},
    store::{ImagePair, ImageStore, StoreError, read_state},
};
use crate::{
    blocking::{self, OffLoop},
    fault,
    tools::Tools,
};

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Ext4(#[from] Ext4Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A new generation's id.
pub fn new_generation() -> GenerationId {
    GenerationId::parse(uuid::Uuid::new_v4().to_string()).expect("a UUID is an image name")
}

#[derive(Debug, Clone)]
pub struct WorkingPair {
    directory: PathBuf,
}

impl WorkingPair {
    pub fn new(working: &Path, device: &DeviceId) -> Self {
        Self {
            directory: working.join(device),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn images(&self) -> ImagePair<PathBuf> {
        ImagePair::in_directory(&self.directory)
    }

    pub fn sandbox_record(&self) -> PathBuf {
        self.directory.join("sandbox.json")
    }

    /// The working pair's record, `None` when the device has no working pair.
    pub fn manifest(&self, off: &OffLoop) -> Result<Option<MachineImageState>, StoreError> {
        read_state(off, &self.directory.join("manifest.json"))
    }

    /// Replaces the record durably.
    pub fn write_manifest(&self, off: &OffLoop, state: &MachineImageState) -> io::Result<()> {
        write_json(off, &self.directory.join("manifest.json"), state)?;
        sync(off, &self.directory)
    }

    /// Publishes the working pair as `device`'s new generation and removes
    /// it; nothing happens without one. Nothing may write the images: each
    /// is checked, an interrupted growth completed, and each synced, then
    /// linked into the generation.
    pub async fn save(&self, tools: &Tools, store: &ImageStore, device: &DeviceId) -> Result<(), SaveError> {
        let pair = self.clone();
        let Some(state) = blocking::run(move |off| pair.manifest(off)).await? else {
            return Ok(());
        };
        let images = self.images();
        let system_bytes = ext4::recover(tools, &images.system).await?;
        let home_bytes = ext4::recover(tools, &images.home).await?;
        let saved = MachineImageState {
            generation: new_generation(),
            system_bytes,
            home_bytes,
            ..state
        };
        let pair = self.clone();
        let store = store.clone();
        let device = device.clone();
        blocking::run(move |off| -> Result<(), SaveError> {
            store.publish(off, &device, &saved, images.as_deref())?;
            fault::point("working-published");
            remove_tree(off, pair.directory())?;
            let parent = pair.directory().parent().expect("a working pair lies in the working directory");
            sync(off, parent)?;
            Ok(())
        })
        .await
    }
}
