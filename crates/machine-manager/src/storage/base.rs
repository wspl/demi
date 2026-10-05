//! Importing the configured base (`images.md` § Import and publication): the
//! manager verifies its image release once, at startup, extracts it into a
//! stage and publishes it under its base version, the SHA-256 of the
//! manifest's bytes. A base already imported under that version must hold
//! the same manifest bytes.
//!
//! ```text
//! <data>/images/configured.json            the configured base and the one before it
//! <data>/images/bases/<baseVersion>/manifest.json, rootfs/
//! ```
//!
//! After each import the manager keeps those two bases and every base a
//! committed generation or a working pair names, and removes the rest
//! (`managed-hosts.md` § Base retention).

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
    #[error("{} is not a valid record of the configured bases: {source}", path.display())]
    Configured {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(transparent)]
    Store(#[from] StoreError),
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

/// Records `base` as the configured base of the store at `images`, and the
/// one configured before it as the previous, then removes every base that
/// neither names and no committed generation or working pair, under
/// `images` and `working`, is pinned to.
pub async fn retain(images: &Path, working: &Path, base: &BaseVersion) -> Result<(), BaseError> {
    let images = images.to_owned();
    let working = working.to_owned();
    let base = base.clone();
    blocking::run(move |off| -> Result<(), BaseError> {
        let path = images.join("configured.json");
        let recorded: Option<Configured> = match fs_err::read(&path) {
            Ok(bytes) => Some(
                serde_json::from_slice(&bytes)
                    .map_err(|source| BaseError::Configured { path: path.clone(), source })?,
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let configured = match recorded {
            Some(recorded) if recorded.current == base => recorded,
            recorded => Configured {
                current: base,
                previous: recorded.map(|recorded| recorded.current),
            },
        };
        write_json(off, &path, &configured)?;
        sync(off, &images)?;
        let mut kept: Vec<BaseVersion> = pinned(off, &images, &working)?;
        kept.push(configured.current);
        kept.extend(configured.previous);
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
            tracing::info!("machines: removed the Cloud base {name}, which nothing needs");
        }
        sync(off, &bases)?;
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
    crate::fault::point("base-staged");
    let extracted = extract(tools, &release, &stage).await;
    let bases = bases.to_owned();
    blocking::run(move |off| {
        let published = extracted.and_then(|()| publish(off, &release, &stage, &bases));
        if published.is_err() {
            // The stage is never visible to a runtime; the next import
            // removes it if this removal fails.
            if let Err(error) = remove_tree(off, &stage) {
                tracing::warn!("machines: {error}");
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
    let stage = bases.join(format!(".base-{}", uuid::Uuid::new_v4()));
    fs_err::create_dir_all(stage.join("rootfs"))?;
    let copied = copy_verified(&image.join(manifest.rootfs.file.name()), &stage, &manifest);
    if let Err(error) = copied {
        if let Err(cleanup) = remove_tree(off, &stage) {
            tracing::warn!("machines: {cleanup}");
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
    blocking::run(move |off| archive::vet(off, &vetted)).await?;
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::{CloudImage, Entry, RUNNER, TINI, digest, entries};

    fn host() -> Architecture {
        Architecture::host().unwrap()
    }

    fn other() -> Architecture {
        match host() {
            Architecture::Arm64 => Architecture::Amd64,
            Architecture::Amd64 => Architecture::Arm64,
        }
    }

    #[tokio::test]
    async fn a_verified_base_is_imported_once_under_its_manifest_digest() {
        let tools = Tools::on_path();
        let image = CloudImage::new(
            &entries(),
            &[RUNNER, TINI, ("/usr/sbin/init", b"tini")],
            host(),
        );
        let bases = tempfile::tempdir().unwrap();
        let version = import(&tools, image.path(), bases.path()).await.unwrap();
        let bytes = std::fs::read(image.path().join("manifest.json")).unwrap();
        assert_eq!(version.as_str(), digest(&bytes));
        let base = bases.path().join(&version);
        assert_eq!(std::fs::read(base.join("manifest.json")).unwrap(), bytes);
        assert_eq!(
            std::fs::read(base.join("rootfs/usr/bin/tini")).unwrap(),
            b"tini"
        );
        assert!(!base.join("rootfs.tar.zst").exists());
        // A second import finds it; a stale stage is removed.
        std::fs::create_dir(bases.path().join(".base-stale")).unwrap();
        assert_eq!(
            import(&tools, image.path(), bases.path()).await.unwrap(),
            version
        );
        let names: Vec<_> = std::fs::read_dir(bases.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [version.as_str()]);
        // The same version with other stored bytes is refused.
        std::fs::write(base.join("manifest.json"), b"{}").unwrap();
        let error = import(&tools, image.path(), bases.path())
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Pinned Cloud image manifest differs");
    }

    #[tokio::test]
    async fn a_release_that_fails_a_check_is_refused_and_leaves_no_stage() {
        let tools = Tools::on_path();
        let bases = tempfile::tempdir().unwrap();
        let cases: Vec<(CloudImage, &str)> = vec![
            (
                {
                    let image = CloudImage::new(&entries(), &[RUNNER, TINI], host());
                    image.edit(|manifest| manifest["rootfs"]["sha256"] = json!("0".repeat(64)));
                    image
                },
                "Cloud root archive integrity mismatch",
            ),
            (
                CloudImage::new(&entries(), &[RUNNER, TINI], other()),
                "architecture differs",
            ),
            (
                CloudImage::new(&entries(), &[RUNNER], host()),
                "lacks /usr/bin/tini",
            ),
            (
                CloudImage::new(
                    &entries(),
                    &[RUNNER, TINI, ("/usr/bin/demi-helper", b"helper")],
                    host(),
                ),
                "No such file",
            ),
            (
                CloudImage::new(&entries(), &[RUNNER, ("/usr/bin/tini", b"other")], host()),
                "integrity mismatch: /usr/bin/tini",
            ),
            (
                {
                    let mut escaping = entries();
                    escaping.push(("usr/bin/escape", Entry::Link("/etc/hostname")));
                    CloudImage::new(
                        &escaping,
                        &[RUNNER, TINI, ("/usr/bin/escape", b"host")],
                        host(),
                    )
                },
                "Invalid image executable path: /usr/bin/escape",
            ),
        ];
        for (image, expected) in cases {
            let error = import(&tools, image.path(), bases.path())
                .await
                .unwrap_err();
            let message = crate::server::chain(&error);
            assert!(message.contains(expected), "{message}");
            let left: Vec<_> = std::fs::read_dir(bases.path()).unwrap().collect();
            assert!(left.is_empty(), "{expected}: {left:?}");
        }
    }

    /// A generation's record on `base`, as the store writes it.
    fn record(path: &Path, base: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let state = json!({
            "generation": "g1",
            "baseVersion": base,
            "resetId": null,
            "systemBytes": 1024,
            "homeBytes": 1024,
        });
        std::fs::write(path, state.to_string()).unwrap();
    }

    /// An upgrade keeps the bases a reset recorded before it may name and
    /// the bases of every Cloud pinned to one, and removes the others.
    #[tokio::test]
    async fn the_configured_the_previous_and_every_pinned_base_stay_and_the_others_go() {
        let data = tempfile::tempdir().unwrap();
        let images = data.path().join("images");
        let working = data.path().join("working");
        let name = |letter: &str| BaseVersion::parse(letter.repeat(64)).unwrap();
        let import = |letter: &str| {
            std::fs::create_dir_all(images.join("bases").join(letter.repeat(64)).join("rootfs")).unwrap();
        };
        for letter in ["a", "c", "d", "e"] {
            import(letter);
        }
        record(&images.join("dev-1/generations/g1/manifest.json"), &"c".repeat(64));
        record(&working.join("dev-2/manifest.json"), &"d".repeat(64));

        retain(&images, &working, &name("a")).await.unwrap();
        // The next release's base is imported, and the manager starts on it.
        import("b");
        retain(&images, &working, &name("b")).await.unwrap();

        let mut left: Vec<String> = std::fs::read_dir(images.join("bases"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap()[..1].to_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["a", "b", "c", "d"]);
        let configured: Configured =
            serde_json::from_slice(&std::fs::read(images.join("configured.json")).unwrap()).unwrap();
        assert_eq!(
            configured,
            Configured {
                current: name("b"),
                previous: Some(name("a")),
            }
        );
        // The same base configured again keeps the record as it is.
        retain(&images, &working, &name("b")).await.unwrap();
        assert!(images.join("bases").join("a".repeat(64)).exists());
    }
}
