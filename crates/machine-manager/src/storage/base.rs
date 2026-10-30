//! Importing the configured base: the manager verifies its image release
//! once, at startup, extracts it into a stage and publishes it under its
//! base version, the SHA-256 of the manifest's bytes. A base already
//! imported under that version must hold the same manifest bytes.
//!
//! ```text
//! <data>/images/configured.json            the configured base and the one before it
//! <data>/images/bases/<baseVersion>/manifest.json, rootfs/
//! ```
//!
//! The import rejects unknown schemas, mismatched hashes or sizes, missing
//! executable entries and an architecture other than its host's. It strips no
//! unexpected input to make a corrupt release appear valid, and never executes
//! image content on the host.
//!
//! The base directory is writable only while it is staged, then exposed
//! read-only as OverlayFS's lower layer.
//!
//! An upgrade imports the next release's image early with `--import`, which
//! takes none of the running manager's locks: a base is an immutable directory
//! named by its version and published atomically, and a running manager reads
//! no base it was not configured with.
//!
//! After each import the manager keeps those two bases and every base a
//! committed generation or a working pair names, and removes the rest.

use std::{
    io::{self, Read, Write},
    os::fd::AsFd,
    path::{Path, PathBuf},
    time::Duration,
};

use demi_machine_manager_protocol::{
    BaseVersion,
    image::{Architecture, CloudImageManifest, INIT_PATH, ManifestError, RUNNER_PATH},
};
use demi_machine_manager_protocol::MachineImageState;
use demi_shared_artifacts::{Digest, Verifier};
use serde::{Deserialize, Serialize};
use rustix::fs::{Mode, OFlags, ResolveFlags};
use sha2::Digest as _;

use super::{
    room,
    archive::{self, ArchiveError},
    durable::{create_private, remove_tree, sync, write_json},
    store::{StoreError, read_state},
};
use crate::{
    blocking::{self, OffLoop},
    tools::{Tool, ToolError, Tools},
};

/// Extraction of a large base can take minutes on a slow disk.
const EXTRACTION_DEADLINE: Duration = Duration::from_secs(300);

/// Executables every image must embed besides its command packages.
const REQUIRED: [&str; 2] = [RUNNER_PATH, INIT_PATH];

#[derive(Debug, thiserror::Error)]
pub enum BaseError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("Cloud image architecture differs from execution host")]
    Architecture,
    #[error("Cloud image manifest lacks {0}")]
    Missing(&'static str),
    #[error("Pinned Cloud image manifest differs")]
    Differs,
    #[error("Cloud root archive integrity mismatch: {0}")]
    Integrity(demi_shared_artifacts::Error),
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("Invalid image executable path: {0}")]
    Escape(String),
    #[error("Cloud executable integrity mismatch: {0}")]
    Executable(String),
    #[error("{} is not a valid record of the store's bases: {source}", path.display())]
    Record {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    NoRoom(#[from] room::NoRoom),
}

impl From<room::RoomError> for BaseError {
    fn from(error: room::RoomError) -> Self {
        match error {
            room::RoomError::NoRoom(refused) => Self::NoRoom(refused),
            room::RoomError::Io(error) => Self::Io(error),
        }
    }
}

/// The release of the runner that the imported base at `base` embeds,
/// which its manifest names.
pub async fn runner_release(base: &Path) -> Result<String, BaseError> {
    let bytes = tokio::fs::read(base.join("manifest.json")).await?;
    let manifest = CloudImageManifest::decode(&bytes)?;
    Ok(manifest.runner.release)
}

/// The configured base and the base configured before it, which a reset
/// recorded before an upgrade may still name.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Configured {
    current: BaseVersion,
    previous: Option<BaseVersion>,
}

/// The base an import beside a running manager last imported, as ahead of
/// an update, kept in the store's `ahead.json` until a manager is configured
/// with it. Since no manager ran it, no Cloud is pinned to it.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Ahead {
    base: BaseVersion,
}

/// Reads the store's JSON record at `path`; none when there is none.
fn read_record<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, BaseError> {
    match fs_err::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|source| BaseError::Record { path: path.to_owned(), source }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Records `base` as the configured base of the store at `images`, and the
/// one configured before it as the previous, then removes every base that
/// neither names, that was not imported ahead, that no committed
/// generation or working pair, under `images` and `working`, is pinned to,
/// and that is not among `running`, the bases whose programs the sandboxes
/// taken over mount (`crate::handover`).
pub async fn retain(
    images: &Path,
    working: &Path,
    base: &BaseVersion,
    running: &[BaseVersion],
) -> Result<(), BaseError> {
    let running = running.to_vec();
    let images = images.to_owned();
    let working = working.to_owned();
    let base = base.clone();
    blocking::run(move |off| -> Result<(), BaseError> {
        let path = images.join("configured.json");
        let recorded: Option<Configured> = read_record(&path)?;
        let configured = match recorded {
            Some(recorded) if recorded.current == base => recorded,
            recorded => Configured {
                current: base,
                previous: recorded.map(|recorded| recorded.current),
            },
        };
        write_json(off, &path, &configured)?;
        // The base imported ahead is configured now, or still ahead.
        let ahead_path = images.join("ahead.json");
        let ahead = match read_record::<Ahead>(&ahead_path)? {
            Some(ahead) if ahead.base == configured.current => {
                fs_err::remove_file(&ahead_path)?;
                None
            }
            ahead => ahead.map(|ahead| ahead.base),
        };
        sync(off, &images)?;
        let mut kept: Vec<BaseVersion> = pinned(off, &images, &working)?;
        kept.push(configured.current);
        kept.extend(running);
        kept.extend(configured.previous);
        kept.extend(ahead);
        let bases = images.join("bases");
        for entry in fs_err::read_dir(&bases)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // A stage is the next import's to remove.
            if name.starts_with('.') || kept.iter().any(|base| base.as_str() == name) {
                continue;
            }
            remove_tree(off, &entry.path())?;
            tracing::info!(event = "cloud_base.removed", %name, "a Cloud base that nothing needs was removed");
        }
        sync(off, &bases)?;
        Ok(())
    })
    .await
}

/// Records `imported`, which an import beside a running manager put in the
/// store at `images`, as the base imported ahead, and removes the one
/// imported ahead before it, which no manager ran and no Cloud is pinned
/// to: one base waits ahead at most. A base a manager is configured with is
/// none imported ahead.
pub async fn imported_ahead(images: &Path, imported: &BaseVersion) -> Result<(), BaseError> {
    let images = images.to_owned();
    let imported = imported.clone();
    blocking::run(move |off| -> Result<(), BaseError> {
        let configured: Option<Configured> = read_record(&images.join("configured.json"))?;
        let ahead_path = images.join("ahead.json");
        let earlier: Option<Ahead> = read_record(&ahead_path)?;
        let ran = |base: &BaseVersion| {
            configured
                .as_ref()
                .is_some_and(|configured| configured.current == *base || configured.previous.as_ref() == Some(base))
        };
        if ran(&imported) {
            if earlier.is_some() {
                fs_err::remove_file(&ahead_path)?;
            }
        } else {
            write_json(off, &ahead_path, &Ahead { base: imported.clone() })?;
        }
        sync(off, &images)?;
        if let Some(earlier) = earlier.filter(|earlier| earlier.base != imported && !ran(&earlier.base)) {
            let path = images.join("bases").join(earlier.base.as_str());
            if path.exists() {
                remove_tree(off, &path)?;
                tracing::info!(event = "cloud_base.removed", name = %earlier.base, "a Cloud base imported ahead was replaced by a newer one");
            }
        }
        Ok(())
    })
    .await
}

/// The bases the committed generations and the working pairs name.
fn pinned(off: &OffLoop, images: &Path, working: &Path) -> Result<Vec<BaseVersion>, BaseError> {
    let mut records = Vec::new();
    for device in fs_err::read_dir(images)? {
        let device = device?;
        if device.file_name() == "bases" || !device.file_type()?.is_dir() {
            continue;
        }
        let generations = device.path().join("generations");
        let entries = match fs_err::read_dir(&generations) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for generation in entries {
            records.push(generation?.path().join("manifest.json"));
        }
    }
    match fs_err::read_dir(working) {
        Ok(devices) => {
            for device in devices {
                records.push(device?.path().join("manifest.json"));
            }
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
        Err(_) => {}
    }
    let mut bases = Vec::new();
    for record in records {
        let state: Option<MachineImageState> = read_state(off, &record)?;
        bases.extend(state.map(|state| state.base_version));
    }
    Ok(bases)
}

/// A release read and checked, not yet imported.
struct Release {
    manifest: CloudImageManifest,
    bytes: Vec<u8>,
    version: BaseVersion,
}

/// Imports the release in `image` into `bases`, unless it is there already,
/// and returns its base version.
pub async fn import(tools: &Tools, image: &Path, bases: &Path) -> Result<BaseVersion, BaseError> {
    let image = image.to_owned();
    let root = bases.to_owned();
    let prepared = blocking::run(move |off| prepare(off, &image, &root)).await?;
    let Some((release, stage)) = prepared.pending else {
        return Ok(prepared.version);
    };
    let extracted = extract(tools, &release, &stage).await;
    let bases = bases.to_owned();
    blocking::run(move |off| {
        let published = extracted.and_then(|()| publish(off, &release, &stage, &bases));
        if published.is_err() {
            // The stage is never visible to a runtime; the next import
            // removes it if this removal fails.
            if let Err(error) = remove_tree(off, &stage) {
                tracing::warn!(event = "stage.remove_failed", kind = "base_publish", %error, "a stage that stays behind could not be removed");
            }
        }
        published.map(|()| release.version)
    })
    .await
}

/// What [`prepare`] found: the version, and the release still to extract
/// with its stage when the base is not imported yet.
struct Prepared {
    version: BaseVersion,
    pending: Option<(Release, PathBuf)>,
}

/// Reads and checks the release, removes stale stages, and copies the
/// archive into a new stage, verifying its size and SHA-256 on the way.
fn prepare(off: &OffLoop, image: &Path, bases: &Path) -> Result<Prepared, BaseError> {
    let bytes = fs_err::read(image.join("manifest.json"))?;
    let version = BaseVersion::parse(format!("{:x}", sha2::Sha256::digest(&bytes)))
        .expect("a SHA-256 in hexadecimal is an image name");
    let manifest = CloudImageManifest::decode(&bytes)?;
    if Architecture::host() != Some(manifest.architecture) {
        return Err(BaseError::Architecture);
    }
    for required in REQUIRED {
        if !manifest.executables.contains_key(required) {
            return Err(BaseError::Missing(required));
        }
    }
    create_private(off, bases)?;
    for entry in fs_err::read_dir(bases)? {
        let entry = entry?;
        // An import is not visible until its final rename.
        if entry.file_name().to_string_lossy().starts_with(".base-") {
            remove_tree(off, &entry.path())?;
        }
    }
    let target = bases.join(&version);
    match fs_err::read(target.join("manifest.json")) {
        Ok(saved) if saved == bytes => {
            return Ok(Prepared {
                version,
                pending: None,
            });
        }
        Ok(_) => return Err(BaseError::Differs),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    // An import frees nothing to make its room: the bases a Cloud stands on
    // stay, and no Cloud's save goes for it.
    let _reserved = room::ensure(off, bases, manifest.rootfs.size, |_| Ok(false))?;
    let stage = bases.join(format!(".base-{}", uuid::Uuid::new_v4()));
    fs_err::create_dir_all(stage.join("rootfs"))?;
    let copied = copy_verified(&image.join(manifest.rootfs.file.name()), &stage, &manifest);
    if let Err(error) = copied {
        if let Err(cleanup) = remove_tree(off, &stage) {
            tracing::warn!(event = "stage.remove_failed", kind = "base_copy", error = %cleanup, "a stage that stays behind could not be removed");
        }
        return Err(error);
    }
    Ok(Prepared {
        version: version.clone(),
        pending: Some((
            Release {
                manifest,
                bytes,
                version,
            },
            stage,
        )),
    })
}

fn copy_verified(
    source: &Path,
    stage: &Path,
    manifest: &CloudImageManifest,
) -> Result<(), BaseError> {
    let expected = Digest {
        size: manifest.rootfs.size,
        sha256: manifest.rootfs.sha256.clone(),
    };
    let mut from = fs_err::File::open(source)?;
    let mut to = fs_err::File::create_new(stage.join(manifest.rootfs.file.name()))?;
    let mut verifier = Verifier::new(&expected);
    let mut buffer = vec![0; 1 << 20];
    loop {
        let count = from.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        verifier
            .update(&buffer[..count])
            .map_err(BaseError::Integrity)?;
        to.write_all(&buffer[..count])?;
    }
    verifier.finish().map_err(BaseError::Integrity)
}

/// Vets the archive's entries, then extracts it with `bsdtar`, which keeps
/// numeric owners, modes with setuid bits, ACLs and extended attributes.
async fn extract(tools: &Tools, release: &Release, stage: &Path) -> Result<(), BaseError> {
    let archive = stage.join(release.manifest.rootfs.file.name());
    let vetted = archive.clone();
    let root = stage.to_owned();
    // The unpacked bytes count as taken until the extraction ends.
    let _reserved = blocking::run(move |off| -> Result<room::Reserved, BaseError> {
        let bytes = archive::vet(off, &vetted)?;
        Ok(room::ensure(off, &root, bytes, |_| Ok(false))?)
    })
    .await?;
    let root = stage.join("rootfs");
    let args = [
        "-xpf".as_ref(),
        archive.as_os_str(),
        "--xattrs".as_ref(),
        "-C".as_ref(),
        root.as_os_str(),
    ];
    tools
        .run(Tool::Bsdtar, args, Some(EXTRACTION_DEADLINE))
        .await?;
    Ok(())
}

/// Checks each embedded executable, then makes the stage the base.
fn publish(off: &OffLoop, release: &Release, stage: &Path, bases: &Path) -> Result<(), BaseError> {
    let root = stage.join("rootfs");
    let directory = fs_err::File::open(&root)?;
    for (path, artifact) in &release.manifest.executables {
        let expected = Digest {
            size: artifact.size,
            sha256: artifact.sha256.clone(),
        };
        verify_executable(directory.file(), path, &expected)?;
    }
    fs_err::remove_file(stage.join(release.manifest.rootfs.file.name()))?;
    fs_err::write(stage.join("manifest.json"), &release.bytes)?;
    rustix::fs::syncfs(directory.file()).map_err(io::Error::from)?;
    sync(off, stage)?;
    fs_err::rename(stage, bases.join(&release.version))?;
    sync(off, bases)?;
    Ok(())
}

/// Opens `path` beneath the extracted root without following a link out of
/// it, and requires a regular file with the expected size and SHA-256.
fn verify_executable(root: &std::fs::File, path: &str, expected: &Digest) -> Result<(), BaseError> {
    let relative = path.trim_start_matches('/');
    let opened = rustix::fs::openat2(
        root.as_fd(),
        relative,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOCTTY,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS,
    );
    let descriptor = match opened {
        Ok(descriptor) => descriptor,
        Err(rustix::io::Errno::XDEV | rustix::io::Errno::LOOP) => {
            return Err(BaseError::Escape(path.to_owned()));
        }
        Err(error) => return Err(crate::linux::failed("opening", Path::new(path), error).into()),
    };
    let mut file = std::fs::File::from(descriptor);
    if !file.metadata()?.is_file() {
        return Err(BaseError::Executable(path.to_owned()));
    }
    let mut verifier = Verifier::new(expected);
    let mut buffer = vec![0; 1 << 20];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        verifier
            .update(&buffer[..count])
            .map_err(|_| BaseError::Executable(path.to_owned()))?;
    }
    verifier
        .finish()
        .map_err(|_| BaseError::Executable(path.to_owned()))
}
