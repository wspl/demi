//! Archive installation: a verified zip archive unpacked into a directory
//! named by its SHA-256, beside a receipt that lets a later process trust
//! the files without downloading them again. A paired device installs into
//! its user's home and the Cloud image build into the image, with the same
//! steps (`images.md` § Root filesystem contents).

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{Digest, Error, InstallLock};

/// The most bytes an installed executable may have: the digest of a larger
/// file stops early.
const EXECUTABLE_BYTES: u64 = 1024 * 1024 * 1024;

/// An archive to install: the HTTPS URL it is downloaded from, the size and
/// SHA-256 it must have, and its executable's path inside it, with `/`
/// between the components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub url: String,
    pub digest: Digest,
    pub executable: String,
}

/// What an installation records beside its files: the SHA-256 of the archive
/// it came from and of its executable.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    archive_hash: String,
    executable_hash: String,
}

/// The installation of `archive` at `directory`: its executable, once the
/// receipt there names the archive and the executable's current SHA-256, or
/// none when nothing is at `directory`. An installation that fails the check
/// is an error rather than a reason to install again or elsewhere: its files
/// changed after it was installed.
pub async fn installed(
    directory: &Path,
    archive: &Archive,
    cancel: &CancellationToken,
) -> Result<Option<PathBuf>, Error> {
    if !tokio::fs::try_exists(directory).await? {
        return Ok(None);
    }
    let invalid = |reason: &str| Error::Installation {
        directory: directory.to_owned(),
        reason: reason.to_owned(),
    };
    let Some(bytes) = crate::receipt::read(directory).await? else {
        return Err(invalid("has no receipt"));
    };
    let receipt: Receipt = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(&format!("has an invalid receipt: {error}")))?;
    let executable = directory.join(&archive.executable);
    let found = match crate::digest(&executable, EXECUTABLE_BYTES, cancel).await {
        Ok(found) => found,
        Err(Error::TooLarge { .. }) => return Err(invalid("fails its integrity check")),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(invalid("fails its integrity check"));
        }
        Err(error) => return Err(error),
    };
    if receipt.archive_hash != archive.digest.sha256 || receipt.executable_hash != found.sha256 {
        return Err(invalid("fails its integrity check"));
    }
    Ok(Some(executable))
}

/// Installs `archive` into the directory of `root` its SHA-256 names and
/// returns the executable; an installation already there is checked, not
/// replaced. Installers of one archive share the lock `<SHA-256>.lock` in
/// `root`, so one downloads and the others find its installation.
pub async fn install_archive(
    client: &reqwest::Client,
    root: &Path,
    archive: &Archive,
    cancel: &CancellationToken,
) -> Result<PathBuf, Error> {
    let sha256 = &archive.digest.sha256;
    tokio::fs::create_dir_all(root).await?;
    let _lock = InstallLock::acquire(&root.join(format!("{sha256}.lock")), cancel).await?;
    let destination = root.join(sha256);
    if let Some(executable) = installed(&destination, archive, cancel).await? {
        return Ok(executable);
    }
    // Everything is staged here and gone with it, whatever happens.
    let temporary = tempfile::Builder::new().prefix(".install-").tempdir_in(root)?;
    let downloaded = temporary.path().join("archive.zip");
    let mut output = tokio::fs::File::create(&downloaded).await?;
    crate::download(client, &archive.url, &archive.digest, &mut output, cancel).await?;
    drop(output);
    let extracted = temporary.path().join("extracted");
    extract_zip(&downloaded, &extracted, cancel).await?;
    publish_installation(&extracted, &destination, archive, cancel).await
}

/// Writes the receipt of `archive`'s files, unpacked at `extracted`, and
/// publishes them as `destination`; returns the executable there.
async fn publish_installation(
    extracted: &Path,
    destination: &Path,
    archive: &Archive,
    cancel: &CancellationToken,
) -> Result<PathBuf, Error> {
    let executable = match crate::digest(&extracted.join(&archive.executable), EXECUTABLE_BYTES, cancel).await {
        Ok(executable) => executable,
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::Archive(format!("holds no {}", archive.executable)));
        }
        Err(error) => return Err(error),
    };
    let receipt = Receipt {
        archive_hash: archive.digest.sha256.clone(),
        executable_hash: executable.sha256,
    };
    crate::receipt::write(extracted, &receipt).await?;
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    crate::publish_directory(extracted, destination).await?;
    Ok(destination.join(&archive.executable))
}

/// Installs into `root`, as [`install_archive`] does after its download, the
/// files of `archive` that a copy unpacked elsewhere holds, named by that
/// copy's `executable`, so that [`installed`] accepts them. For a test given
/// an installed release's executable, such as the Chrome suite's
/// `DEMI_TEST_CHROME`: nothing is downloaded, and the files are hard links
/// where the system allows them, copies otherwise.
#[cfg(feature = "testing")]
pub async fn install_unpacked(
    root: &Path,
    archive: &Archive,
    executable: &Path,
    cancel: &CancellationToken,
) -> Result<PathBuf, Error> {
    let unpacked = Path::new(&archive.executable)
        .components()
        .try_fold(executable, |path, _| path.parent())
        .filter(|unpacked| unpacked.join(&archive.executable) == executable)
        .ok_or_else(|| {
            Error::Archive(format!("{} is not its {}", executable.display(), archive.executable))
        })?
        .to_owned();
    tokio::fs::create_dir_all(root).await?;
    let temporary = tempfile::Builder::new().prefix(".install-").tempdir_in(root)?;
    let extracted = temporary.path().join("extracted");
    let linked = extracted.clone();
    tokio::task::spawn_blocking(move || link_tree(&unpacked, &linked))
        .await
        .map_err(std::io::Error::other)??;
    publish_installation(&extracted, &root.join(&archive.digest.sha256), archive, cancel).await
}

/// Makes `destination` a tree like `source`: new directories, the same
/// symbolic links, and each file hard-linked, or copied.
#[cfg(feature = "testing")]
fn link_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::create_dir(destination)?;
    for entry in std::fs::read_dir(source).map_err(|error| at(source, error))? {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            link_tree(&from, &to)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(&from)?, &to)?;
            #[cfg(not(unix))]
            return Err(std::io::Error::other(format!("{} is a symbolic link", from.display())));
        } else if std::fs::hard_link(&from, &to).is_err() {
            // Every system refuses a hard link across filesystems, and Linux
            // one to another user's file (`protected_hardlinks`); a copy has
            // the same bytes, and fails on its own if the file is unreadable.
            std::fs::copy(&from, &to).map_err(|error| at(&from, error))?;
        }
    }
    Ok(())
}

/// `error`, naming the file of the unpacked copy it happened to.
#[cfg(feature = "testing")]
fn at(path: &Path, error: std::io::Error) -> std::io::Error {
    std::io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

/// Whether the zip archive at `archive` holds a file at `path`, such as the
/// executable a release record names; only the archive's directory is read.
pub async fn zip_holds(archive: &Path, path: &str) -> Result<bool, Error> {
    let archive = archive.to_owned();
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let unreadable = |error: zip::result::ZipError| Error::Archive(format!("cannot be read: {error}"));
        let mut archive = zip::ZipArchive::new(File::open(archive)?).map_err(unreadable)?;
        let Some(index) = archive.index_for_name(&path) else {
            return Ok(false);
        };
        let entry = archive.by_index_raw(index).map_err(unreadable)?;
        Ok(entry.is_file())
    })
    .await
    .map_err(std::io::Error::other)?
}

/// zip checks each entry's path and CRC but has no cancellation of its own;
/// the reader it reads through stops when cancelled.
struct Cancellable {
    file: File,
    cancel: CancellationToken,
}

impl Read for Cancellable {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.is_cancelled() {
            // Not `Interrupted`: the standard library's readers retry that
            // kind, so extraction would spin instead of stopping.
            return Err(std::io::Error::other("extraction cancelled"));
        }
        self.file.read(buffer)
    }
}

impl Seek for Cancellable {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(position)
    }
}

/// Extracts the zip archive at `archive` into `destination` on the blocking
/// pool. The call returns only once extraction has stopped, also when
/// cancelled, so the caller alone owns what was extracted.
async fn extract_zip(archive: &Path, destination: &Path, cancel: &CancellationToken) -> Result<(), Error> {
    let archive = archive.to_owned();
    let destination = destination.to_owned();
    let reader_cancel = cancel.clone();
    let extracted = tokio::task::spawn_blocking(move || {
        let reader = Cancellable {
            file: File::open(archive)?,
            cancel: reader_cancel,
        };
        let unusable = |error: zip::result::ZipError| Error::Archive(format!("cannot be extracted: {error}"));
        let mut archive = zip::ZipArchive::new(reader).map_err(unusable)?;
        archive.extract(destination).map_err(unusable)
    })
    .await
    .map_err(std::io::Error::other)?;
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    extracted
}
