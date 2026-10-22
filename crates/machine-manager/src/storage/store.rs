//! The immutable image store: each device's committed generations and the
//! pointer to the current one.
//!
//! ```text
//! <data>/images/<device>/current.json               the committed generation
//! <data>/images/<device>/generations/<generation>/  manifest.json, system.ext4, home.ext4
//! ```
//!
//! Publication links images into the new generation instead of copying them:
//! every source is an image nothing writes any more.

use std::{
    io,
    path::{Path, PathBuf},
};

use demi_machine_manager_protocol::{
    DeviceId, GenerationId, MachineImageState, SandboxMachine, Volume,
};

use super::durable::{remove_tree, sync, write_json};
use crate::blocking::OffLoop;

/// The file name of `volume`'s image in a generation, a working pair or a
/// stage: `system.ext4` or `home.ext4`.
pub fn image_file(volume: Volume) -> String {
    format!("{volume}.ext4")
}

/// A value for each of a device's two volumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePair<T> {
    pub system: T,
    pub home: T,
}

impl<T> ImagePair<T> {
    pub fn get(&self, volume: Volume) -> &T {
        match volume {
            Volume::System => &self.system,
            Volume::Home => &self.home,
        }
    }
}

impl ImagePair<PathBuf> {
    /// The images named `system.ext4` and `home.ext4` in `directory`.
    pub fn in_directory(directory: &Path) -> Self {
        Self {
            system: directory.join(image_file(Volume::System)),
            home: directory.join(image_file(Volume::Home)),
        }
    }

    pub fn as_deref(&self) -> ImagePair<&Path> {
        ImagePair {
            system: &self.system,
            home: &self.home,
        }
    }
}

/// Which committed generations a publication keeps beside the new
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// The generation the new one replaces stays, there when the new one
    /// turns out damaged: a user's Cloud, and a save the manager makes on
    /// its own, which names no machine.
    Previous,
    /// The new generation alone: the Claude Code machine, whose contents
    /// are downloaded or signed in again.
    Current,
}

impl Retention {
    /// The retention of `machine`'s generations.
    pub fn of(machine: SandboxMachine) -> Self {
        match machine {
            SandboxMachine::Cloud => Self::Previous,
            SandboxMachine::ClaudeCode => Self::Current,
        }
    }
}

/// A failed read or publication. File errors name their path.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Io(#[from] io::Error),
    /// A record that does not decode is an error; nothing repairs it.
    #[error("{} is not a valid generation record: {source}", path.display())]
    Corrupt {
        path: PathBuf,
        source: serde_json::Error,
    },
}

/// Reads a generation record at `path`; `None` when there is none.
pub fn read_state(_: &OffLoop, path: &Path) -> Result<Option<MachineImageState>, StoreError> {
    let bytes = match fs_err::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let state = serde_json::from_slice(&bytes).map_err(|source| StoreError::Corrupt {
        path: path.to_owned(),
        source,
    })?;
    Ok(Some(state))
}

/// The committed generations under `<data>/images`.
#[derive(Debug, Clone)]
pub struct ImageStore {
    root: PathBuf,
}

impl ImageStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Imported bases: `<data>/images/bases`.
    pub fn bases(&self) -> PathBuf {
        self.root.join("bases")
    }

    /// A device's committed generation, `None` before its first.
    pub fn read(
        &self,
        off: &OffLoop,
        device: &DeviceId,
    ) -> Result<Option<MachineImageState>, StoreError> {
        read_state(off, &self.root.join(device).join("current.json"))
    }

    /// The images of one of a device's generations.
    pub fn images(&self, device: &DeviceId, generation: &GenerationId) -> ImagePair<PathBuf> {
        ImagePair::in_directory(&self.root.join(device).join("generations").join(generation))
    }

    /// Commits `state`'s generation with `sources` as its images, then keeps
    /// only it and, as `retention` says, the generation it replaces. Each
    /// source is linked, so it must be on this filesystem and must never be
    /// written again; its producer has synced it. A failure before the new
    /// `current.json` leaves the committed generation as it was and no stage
    /// behind.
    pub fn publish(
        &self,
        off: &OffLoop,
        device: &DeviceId,
        state: &MachineImageState,
        sources: ImagePair<&Path>,
        retention: Retention,
    ) -> Result<(), StoreError> {
        let previous = self.read(off, device)?;
        let directory = self.root.join(device);
        let generations = directory.join("generations");
        fs_err::create_dir_all(&generations)?;
        sync(off, &self.root)?;
        sync(off, &directory)?;
        let stage = generations.join(format!(".publish-{}", uuid::Uuid::new_v4()));
        fs_err::create_dir(&stage)?;
        let staged = stage_generation(off, &stage, state, &sources).and_then(|()| {
            fs_err::rename(&stage, generations.join(&state.generation))
        });
        if let Err(error) = staged {
            // A failure to remove the stage is logged beside the first error,
            // which the caller acts on; the next publication removes it.
            if let Err(cleanup) = remove_tree(off, &stage) {
                tracing::warn!(event = "stage.remove_failed", kind = "generation", error = %cleanup, "a stage that stays behind could not be removed");
            }
            return Err(error.into());
        }
        sync(off, &generations)?;
        write_json(off, &directory.join("current.json"), state)?;
        sync(off, &directory)?;
        // Keep the fallback generation where the retention has one; older
        // ones and stale stages go, which removes only their links to images
        // a newer generation may share.
        let fallback = previous.filter(|_| retention == Retention::Previous);
        for entry in fs_err::read_dir(&generations)? {
            let entry = entry?;
            let name = entry.file_name();
            let keep = name == state.generation.as_str()
                || fallback
                    .as_ref()
                    .is_some_and(|previous| name == previous.generation.as_str());
            if !keep && entry.file_type()?.is_dir() {
                remove_tree(off, &entry.path())?;
            }
        }
        sync(off, &generations)?;
        Ok(())
    }
}

fn stage_generation(
    off: &OffLoop,
    stage: &Path,
    state: &MachineImageState,
    sources: &ImagePair<&Path>,
) -> io::Result<()> {
    for volume in Volume::ALL {
        fs_err::hard_link(sources.get(volume), stage.join(image_file(volume)))?;
    }
    write_json(off, &stage.join("manifest.json"), state)?;
    sync(off, stage)
}
