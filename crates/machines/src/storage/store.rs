//! The immutable image store (`managed-hosts.md` § Save a generation): each
//! device's committed generations and the pointer to the current one.
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

use demi_machines_protocol::{DeviceId, GenerationId, MachineImageState, Volume};

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

/// A failed read or publication. File errors name their path.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Io(#[from] io::Error),
    /// A record that does not decode is an error; nothing repairs it.
    #[error("{} is not a valid generation record: {source}", path.display())]
    Corrupt { path: PathBuf, source: serde_json::Error },
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
    pub fn read(&self, off: &OffLoop, device: &DeviceId) -> Result<Option<MachineImageState>, StoreError> {
        read_state(off, &self.root.join(device).join("current.json"))
    }

    /// The images of one of a device's generations.
    pub fn images(&self, device: &DeviceId, generation: &GenerationId) -> ImagePair<PathBuf> {
        ImagePair::in_directory(&self.root.join(device).join("generations").join(generation))
    }

    /// Commits `state`'s generation with `sources` as its images, then keeps
    /// only it and the generation it replaces. Each source is linked, so it
    /// must be on this filesystem and must never be written again; its
    /// producer has synced it. A failure before the new `current.json` leaves
    /// the committed generation as it was and no stage behind.
    pub fn publish(
        &self,
        off: &OffLoop,
        device: &DeviceId,
        state: &MachineImageState,
        sources: ImagePair<&Path>,
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
            crate::fault::point("generation-staged");
            fs_err::rename(&stage, generations.join(&state.generation))
        });
        if let Err(error) = staged {
            // A failure to remove the stage is logged beside the first error,
            // which the caller acts on; the next publication removes it.
            if let Err(cleanup) = remove_tree(off, &stage) {
                tracing::warn!("machines: {cleanup}");
            }
            return Err(error.into());
        }
        sync(off, &generations)?;
        crate::fault::point("generation-renamed");
        write_json(off, &directory.join("current.json"), state)?;
        sync(off, &directory)?;
        // Keep the fallback generation; older ones and stale stages go, which
        // removes only their links to images a newer generation may share.
        for entry in fs_err::read_dir(&generations)? {
            let entry = entry?;
            let name = entry.file_name();
            let keep = name == state.generation.as_str()
                || previous
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

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use demi_machines_protocol::BaseVersion;

    use super::*;

    fn state(generation: &str) -> MachineImageState {
        MachineImageState {
            generation: GenerationId::parse(generation).unwrap(),
            base_version: BaseVersion::parse("base").unwrap(),
            reset_id: None,
            system_bytes: NonZeroU64::new(1024).unwrap(),
            home_bytes: NonZeroU64::new(1024).unwrap(),
        }
    }

    fn generations(root: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(root.join("device/generations"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_failed_publication_keeps_the_committed_pair_and_only_two_generations_remain() {
        let off = OffLoop::in_test();
        let directory = tempfile::tempdir().unwrap();
        let store = ImageStore::new(directory.path().join("images"));
        std::fs::create_dir(directory.path().join("images")).unwrap();
        let device = DeviceId::parse("device").unwrap();
        let source = directory.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let sources = ImagePair::in_directory(&source);
        std::fs::write(&sources.system, "system").unwrap();
        std::fs::write(&sources.home, "home").unwrap();
        let pair = sources.as_deref();
        assert_eq!(store.read(&off, &device).unwrap(), None);
        store.publish(&off, &device, &state("first"), pair.clone()).unwrap();

        std::fs::remove_file(&sources.home).unwrap();
        assert!(store.publish(&off, &device, &state("partial"), pair.clone()).is_err());
        assert_eq!(store.read(&off, &device).unwrap(), Some(state("first")));
        assert_eq!(generations(&store.root), ["first"]);

        // The generation owns its images: the sources' removal left them.
        let committed = store.images(&device, &state("first").generation);
        assert_eq!(std::fs::read_to_string(&committed.home).unwrap(), "home");
        for volume in Volume::ALL {
            std::fs::copy(committed.get(volume), sources.get(volume)).unwrap();
        }
        store.publish(&off, &device, &state("second"), pair.clone()).unwrap();
        std::fs::remove_file(&sources.system).unwrap();
        std::fs::remove_file(&sources.home).unwrap();
        let second = store.images(&device, &state("second").generation);
        store.publish(&off, &device, &state("third"), second.as_deref()).unwrap();
        assert_eq!(generations(&store.root), ["second", "third"]);
        assert_eq!(store.read(&off, &device).unwrap(), Some(state("third")));
    }

    #[test]
    #[cfg(unix)]
    fn publication_links_its_sources() {
        use std::os::unix::fs::MetadataExt;
        let off = OffLoop::in_test();
        let directory = tempfile::tempdir().unwrap();
        let store = ImageStore::new(directory.path().join("images"));
        std::fs::create_dir(directory.path().join("images")).unwrap();
        let device = DeviceId::parse("device").unwrap();
        let sources = ImagePair::in_directory(directory.path());
        std::fs::write(&sources.system, "system").unwrap();
        std::fs::write(&sources.home, "home").unwrap();
        store.publish(&off, &device, &state("first"), sources.as_deref()).unwrap();
        let committed = store.images(&device, &state("first").generation);
        for volume in Volume::ALL {
            let source = std::fs::metadata(sources.get(volume)).unwrap();
            let image = std::fs::metadata(committed.get(volume)).unwrap();
            assert_eq!((source.dev(), source.ino()), (image.dev(), image.ino()));
        }
    }

    #[test]
    fn a_corrupt_record_is_an_error() {
        let off = OffLoop::in_test();
        let directory = tempfile::tempdir().unwrap();
        let store = ImageStore::new(directory.path().to_owned());
        let device = DeviceId::parse("device").unwrap();
        std::fs::create_dir(directory.path().join("device")).unwrap();
        std::fs::write(directory.path().join("device/current.json"), r#"{"generation":"g"}"#).unwrap();
        assert!(matches!(store.read(&off, &device), Err(StoreError::Corrupt { .. })));
    }
}
