//! Archive installation: a verified zip archive unpacked into a directory
//! named by its SHA-256, beside a receipt that lets a later process trust
//! the files without downloading them again. A runner installs a command
//! package's resources into its artifact cache and the Cloud image build
//! into the image, with the same steps (`native-runtime.md` § Install the
//! selected package, `images.md` § Root filesystem contents).

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{Digest, Error, InstallLock};

/// The most bytes an installed entry may have: the digest of a larger file
/// stops early.
const ENTRY_BYTES: u64 = 1024 * 1024 * 1024;

/// An archive to install: the size and SHA-256 it must have, and its entry,
/// the file its user starts, as a path inside it with `/` between the
/// components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub digest: Digest,
    pub entry: String,
}

/// What an installation records beside its files: the SHA-256 of the archive
/// it came from and of its entry.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    archive_hash: String,
    entry_hash: String,
}

/// The receipt at `directory`, or none when nothing is there.
async fn receipt(directory: &Path) -> Result<Option<Receipt>, Error> {
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
    let receipt = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(&format!("has an invalid receipt: {error}")))?;
    Ok(Some(receipt))
}

/// The entry of the installation of `archive` at `directory` that the
/// installer published, trusted unread: the receipt there names the
/// archive. None when nothing is at `directory`; another receipt is an
/// error. For a directory only its installer writes, such as a runner's
/// private cache, where reading the whole entry at each use would cost
/// every start.
pub async fn recorded(directory: &Path, archive: &Archive) -> Result<Option<PathBuf>, Error> {
    let Some(receipt) = receipt(directory).await? else {
        return Ok(None);
    };
    if receipt.archive_hash != archive.digest.sha256 {
        return Err(Error::Installation {
            directory: directory.to_owned(),
            reason: "holds another archive".to_owned(),
        });
    }
    Ok(Some(directory.join(&archive.entry)))
}

/// The installation of `archive` at `directory`: its entry, once the
/// receipt there names the archive and the entry's current SHA-256, or
/// none when nothing is at `directory`. An installation that fails the check
/// is an error rather than a reason to install again or elsewhere: its files
/// changed after it was installed.
pub async fn installed(
    directory: &Path,
    archive: &Archive,
    cancel: &CancellationToken,
) -> Result<Option<PathBuf>, Error> {
    let Some(receipt) = receipt(directory).await? else {
        return Ok(None);
    };
    let invalid = |reason: &str| Error::Installation {
        directory: directory.to_owned(),
        reason: reason.to_owned(),
    };
    let entry = directory.join(&archive.entry);
    let found = match crate::digest(&entry, ENTRY_BYTES, cancel).await {
        Ok(found) => found,
        Err(Error::TooLarge { .. }) => return Err(invalid("fails its integrity check")),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(invalid("fails its integrity check"));
        }
        Err(error) => return Err(error),
    };
    if receipt.archive_hash != archive.digest.sha256 || receipt.entry_hash != found.sha256 {
        return Err(invalid("fails its integrity check"));
    }
    Ok(Some(entry))
}

/// What [`install_archive`] finds in its root.
pub enum ArchiveInstall {
    /// The installation was there already: its entry.
    Installed(PathBuf),
    /// Nothing was there: the caller writes the archive's verified bytes to
    /// [`Unpacking::archive`], then [`Unpacking::finish`]es.
    Unpack(Unpacking),
}

/// An installation of one archive under way. It holds the archive's install
/// lock, and its temporary directory goes with it, whatever happens.
pub struct Unpacking {
    _lock: InstallLock,
    temporary: tempfile::TempDir,
    destination: PathBuf,
    archive: Archive,
}

/// Starts installing `archive` into the directory of `root` its SHA-256
/// names; an installation already there is checked, not replaced.
/// Installers of one archive share the lock `<SHA-256>.lock` in `root`,
/// so one fetches the archive and the others find its installation.
pub async fn install_archive(
    root: &Path,
    archive: &Archive,
    cancel: &CancellationToken,
) -> Result<ArchiveInstall, Error> {
    let sha256 = &archive.digest.sha256;
    tokio::fs::create_dir_all(root).await?;
    let lock = InstallLock::acquire(&root.join(format!("{sha256}.lock")), cancel).await?;
    let destination = root.join(sha256);
    if let Some(entry) = installed(&destination, archive, cancel).await? {
        return Ok(ArchiveInstall::Installed(entry));
    }
    let temporary = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(root)?;
    Ok(ArchiveInstall::Unpack(Unpacking {
        _lock: lock,
        temporary,
        destination,
        archive: archive.clone(),
    }))
}

impl Unpacking {
    /// Where the caller writes the archive, checked against its declared
    /// size and SHA-256 as it is written, such as by [`crate::download`].
    pub fn archive(&self) -> PathBuf {
        self.temporary.path().join("archive.zip")
    }

    /// Unpacks the archive the caller wrote, writes the receipt and
    /// publishes the installation; returns its entry.
    pub async fn finish(self, cancel: &CancellationToken) -> Result<PathBuf, Error> {
        let extracted = self.temporary.path().join("extracted");
        extract_zip(&self.archive(), &extracted, cancel).await?;
        publish_installation(&extracted, &self.destination, &self.archive, cancel).await
    }
}

/// Writes the receipt of `archive`'s files, unpacked at `extracted`, and
/// publishes them as `destination`; returns the entry there.
async fn publish_installation(
    extracted: &Path,
    destination: &Path,
    archive: &Archive,
    cancel: &CancellationToken,
) -> Result<PathBuf, Error> {
    let entry = match crate::digest(&extracted.join(&archive.entry), ENTRY_BYTES, cancel).await {
        Ok(entry) => entry,
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::Archive(format!("holds no {}", archive.entry)));
        }
        Err(error) => return Err(error),
    };
    let receipt = Receipt {
        archive_hash: archive.digest.sha256.clone(),
        entry_hash: entry.sha256,
    };
    crate::receipt::write(extracted, &receipt).await?;
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    crate::publish_directory(extracted, destination).await?;
    Ok(destination.join(&archive.entry))
}

/// Installs into `root`, as [`install_archive`] does once the archive is
/// written, the files of `archive` that a copy unpacked elsewhere holds,
/// named by that copy's `entry`, so that [`installed`] accepts them. For a
/// test given an installed release's entry, such as the Chrome suite's
/// `DEMI_TEST_CHROME`: nothing is downloaded, and the files are hard links
/// where the system allows them, copies otherwise.
#[cfg(feature = "testing")]
pub async fn install_unpacked(
    root: &Path,
    archive: &Archive,
    entry: &Path,
    cancel: &CancellationToken,
) -> Result<PathBuf, Error> {
    let unpacked = Path::new(&archive.entry)
        .components()
        .try_fold(entry, |path, _| path.parent())
        .filter(|unpacked| unpacked.join(&archive.entry) == entry)
        .ok_or_else(|| Error::Archive(format!("{} is not its {}", entry.display(), archive.entry)))?
        .to_owned();
    tokio::fs::create_dir_all(root).await?;
    let temporary = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(root)?;
    let extracted = temporary.path().join("extracted");
    let linked = extracted.clone();
    tokio::task::spawn_blocking(move || link_tree(&unpacked, &linked))
        .await
        .map_err(std::io::Error::other)??;
    publish_installation(
        &extracted,
        &root.join(&archive.digest.sha256),
        archive,
        cancel,
    )
    .await
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
            return Err(std::io::Error::other(format!(
                "{} is a symbolic link",
                from.display()
            )));
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
/// entry a release record names; only the archive's directory is read.
pub async fn zip_holds(archive: &Path, path: &str) -> Result<bool, Error> {
    let archive = archive.to_owned();
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let unreadable =
            |error: zip::result::ZipError| Error::Archive(format!("cannot be read: {error}"));
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
async fn extract_zip(
    archive: &Path,
    destination: &Path,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let archive = archive.to_owned();
    let destination = destination.to_owned();
    let reader_cancel = cancel.clone();
    let extracted = tokio::task::spawn_blocking(move || {
        let reader = Cancellable {
            file: File::open(archive)?,
            cancel: reader_cancel,
        };
        let unusable =
            |error: zip::result::ZipError| Error::Archive(format!("cannot be extracted: {error}"));
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
