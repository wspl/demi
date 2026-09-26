//! Importing the configured base (`images.md` § Import and publication): the
//! manager verifies its image release once, at startup, extracts it into a
//! stage and publishes it under its base version, the SHA-256 of the
//! manifest's bytes. A base already imported under that version must hold
//! the same manifest bytes.
//!
//! ```text
//! <data>/images/bases/<baseVersion>/manifest.json, rootfs/
//! ```

use std::{
    io::{self, Read, Write},
    os::fd::AsFd,
    path::{Path, PathBuf},
    time::Duration,
};

use demi_artifact::{Digest, Verifier};
use demi_machines_protocol::{
    BaseVersion,
    image::{Architecture, CloudImageManifest, INIT_PATH, ManifestError, RUNNER_PATH},
};
use rustix::fs::{Mode, OFlags, ResolveFlags};
use sha2::Digest as _;

use super::{
    archive::{self, ArchiveError},
    durable::{create_private, remove_tree, sync},
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
    Integrity(demi_artifact::Error),
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("Invalid image executable path: {0}")]
    Escape(String),
    #[error("Cloud executable integrity mismatch: {0}")]
    Executable(String),
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

fn copy_verified(source: &Path, stage: &Path, manifest: &CloudImageManifest) -> Result<(), BaseError> {
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
        verifier.update(&buffer[..count]).map_err(BaseError::Integrity)?;
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
    tools.run(Tool::Bsdtar, args, Some(EXTRACTION_DEADLINE)).await?;
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
        let image = CloudImage::new(&entries(), &[RUNNER, TINI, ("/usr/sbin/init", b"tini")], host());
        let bases = tempfile::tempdir().unwrap();
        let version = import(&tools, image.path(), bases.path()).await.unwrap();
        let bytes = std::fs::read(image.path().join("manifest.json")).unwrap();
        assert_eq!(version.as_str(), digest(&bytes));
        let base = bases.path().join(&version);
        assert_eq!(std::fs::read(base.join("manifest.json")).unwrap(), bytes);
        assert_eq!(std::fs::read(base.join("rootfs/usr/bin/tini")).unwrap(), b"tini");
        assert!(!base.join("rootfs.tar.zst").exists());
        // A second import finds it; a stale stage is removed.
        std::fs::create_dir(bases.path().join(".base-stale")).unwrap();
        assert_eq!(import(&tools, image.path(), bases.path()).await.unwrap(), version);
        let names: Vec<_> = std::fs::read_dir(bases.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [version.as_str()]);
        // The same version with other stored bytes is refused.
        std::fs::write(base.join("manifest.json"), b"{}").unwrap();
        let error = import(&tools, image.path(), bases.path()).await.unwrap_err();
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
            (CloudImage::new(&entries(), &[RUNNER, TINI], other()), "architecture differs"),
            (CloudImage::new(&entries(), &[RUNNER], host()), "lacks /usr/bin/tini"),
            (CloudImage::new(&entries(), &[RUNNER, TINI, ("/usr/bin/demi-helper", b"helper")], host()), "No such file"),
            (CloudImage::new(&entries(), &[RUNNER, ("/usr/bin/tini", b"other")], host()), "integrity mismatch: /usr/bin/tini"),
            (
                {
                    let mut escaping = entries();
                    escaping.push(("usr/bin/escape", Entry::Link("/etc/hostname")));
                    CloudImage::new(&escaping, &[RUNNER, TINI, ("/usr/bin/escape", b"host")], host())
                },
                "Invalid image executable path: /usr/bin/escape",
            ),
        ];
        for (image, expected) in cases {
            let error = import(&tools, image.path(), bases.path()).await.unwrap_err();
            let message = crate::server::chain(&error);
            assert!(message.contains(expected), "{message}");
            let left: Vec<_> = std::fs::read_dir(bases.path()).unwrap().collect();
            assert!(left.is_empty(), "{expected}: {left:?}");
        }
    }
}
