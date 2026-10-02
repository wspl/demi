//! The verified artifact cache (`native-runtime.md` § Install the selected
//! package): an executable as one file per digest, published only once its
//! size and SHA-256 match what the descriptor declares, and a resource as
//! its archive unpacked into a directory per digest with a receipt; both
//! reused unread from then on. Before it downloads an artifact, it takes the
//! copy the Host's image preinstalled (§ Preinstalled artifacts), which it
//! checks the same way the first time the process needs it. The service
//! registry starts one install per service at a time, so concurrent callers
//! share it; runners that share the cache publish one copy of each artifact.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::SystemTime,
};

use demi_command_protocol::{PackageArtifact, ResourceArtifact};
use demi_runner_protocol::wire::InstallArtifact;
use demi_shared_artifacts::{
    Archive, ArchiveInstall, Digest, Mode, Permissions, Publication, Staged,
};
use tokio::io::AsyncWrite;
use tokio_util::sync::CancellationToken;

use demi_runner_process::private_files::chmod;

use crate::installs::{Installing, Installs};
use crate::{ArtifactResolver, ArtifactSource, RuntimeError};

/// What this process found when it checked the image's copy of each digest:
/// the executable or the resource's entry, or none when the copy failed its
/// check.
type Checked = HashMap<String, Option<PathBuf>>;

pub struct ArtifactCache {
    root: PathBuf,
    /// Where the Host's image preinstalls command artifacts, each in a
    /// directory named by its SHA-256. Nothing is there on a paired device.
    image: Option<PathBuf>,
    checked: Mutex<Checked>,
    http: reqwest::Client,
    installs: Installs,
}

/// Whose artifact an install obtains, for the installs it reports.
#[derive(Clone, Copy)]
pub struct ForPackage<'a> {
    /// The package, such as `demi.browser`.
    pub id: &'a str,
    pub resolver: &'a dyn ArtifactResolver,
}

impl ArtifactCache {
    /// A cache in `root` that takes the artifacts preinstalled in `image`,
    /// when given, before it downloads one, and reports its downloads in
    /// `installs`.
    pub async fn new(
        root: PathBuf,
        image: Option<PathBuf>,
        installs: Installs,
    ) -> Result<Self, RuntimeError> {
        tokio::fs::create_dir_all(&root).await?;
        chmod(&root, 0o700).await?;
        Ok(Self {
            root,
            image,
            checked: Mutex::default(),
            // The pinned descriptor decides what is installed, so a backend
            // on plain HTTP may serve its artifacts itself.
            http: demi_shared_artifacts::client_allowing_http()?,
            installs,
        })
    }

    /// The executable for `artifact`: the cached one, else the image's copy
    /// that matches it, else one downloaded into the cache. A cached entry
    /// that is not a regular file of the declared size fails the install.
    pub async fn install(
        &self,
        artifact: &PackageArtifact,
        package: ForPackage<'_>,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, RuntimeError> {
        let destination = self.root.join(&artifact.sha256);
        if let Some(cached) = cached(&destination, artifact).await? {
            return Ok(cached);
        }
        let expected = digest_of(artifact);
        attempts(cancel, || async {
            if let Some(executable) = self
                .preinstalled(ImageCopy::Executable(&expected), cancel)
                .await?
            {
                return Ok(executable);
            }
            let installing =
                self.installs
                    .start(package.id, InstallArtifact::Program, artifact.size);
            self.download(artifact, &destination, package, &installing, cancel)
                .await?;
            Ok(destination.clone())
        })
        .await
    }

    /// The entry of `resource`, titled `title`: the cached installation,
    /// else the image's that matches it, else its archive downloaded and
    /// unpacked into the cache.
    pub async fn install_resource(
        &self,
        title: &str,
        resource: &ResourceArtifact,
        package: ForPackage<'_>,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, RuntimeError> {
        let archive = Archive {
            digest: digest_of(&resource.archive()),
            entry: resource.entry.clone(),
        };
        let directory = self.root.join(&resource.sha256);
        if let Some(entry) = demi_shared_artifacts::recorded(&directory, &archive).await? {
            return Ok(entry);
        }
        attempts(cancel, || async {
            if let Some(entry) = self
                .preinstalled(ImageCopy::Resource(&archive), cancel)
                .await?
            {
                return Ok(entry);
            }
            let unpacking =
                match demi_shared_artifacts::install_archive(&self.root, &archive, cancel).await? {
                    // Another runner sharing the cache installed it meanwhile.
                    ArchiveInstall::Installed(entry) => return Ok(entry),
                    ArchiveInstall::Unpack(unpacking) => unpacking,
                };
            let installing = self.installs.start(
                package.id,
                InstallArtifact::Resource {
                    title: title.to_owned(),
                },
                resource.size,
            );
            let mut output = tokio::fs::File::create(unpacking.archive()).await?;
            self.fetch(
                &resource.archive(),
                &mut output,
                package,
                &installing,
                cancel,
            )
            .await?;
            drop(output);
            installing.unpacking();
            Ok(unpacking.finish(cancel).await?)
        })
        .await
    }

    /// The image's `copy`, when it matches. The process checks each copy
    /// once, the first time it needs it: one that matched is used unread
    /// from then on, and one that did not is written to the Host log and
    /// passed over. A check that was cancelled or ran out of open files
    /// decides nothing.
    async fn preinstalled(
        &self,
        copy: ImageCopy<'_>,
        cancel: &CancellationToken,
    ) -> Result<Option<PathBuf>, RuntimeError> {
        let Some(image) = &self.image else {
            return Ok(None);
        };
        let sha256 = match copy {
            ImageCopy::Executable(expected) => &expected.sha256,
            ImageCopy::Resource(archive) => &archive.digest.sha256,
        };
        if let Some(checked) = self.checked().get(sha256) {
            return Ok(checked.clone());
        }
        let directory = image.join(sha256);
        let found = match copy {
            ImageCopy::Executable(expected) => check(&directory, expected, cancel).await,
            ImageCopy::Resource(archive) => image_resource(&directory, archive, cancel).await,
        };
        let checked = match found {
            Ok(found) => Some(found),
            Err(_) if cancel.is_cancelled() => return Err(RuntimeError::Cancelled),
            Err(error) if out_of_files(&error) => return Err(error),
            // The image holds no copy, as on every paired device.
            Err(RuntimeError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => {
                tracing::warn!(
                    "preinstalled artifact in {} not used, downloading it: {error}",
                    directory.display()
                );
                None
            }
        };
        self.checked().insert(sha256.clone(), checked.clone());
        Ok(checked)
    }

    fn checked(&self) -> MutexGuard<'_, Checked> {
        self.checked.lock().expect("the checked copies are intact")
    }

    /// Fetches the executable into a staged file beside `destination` and
    /// publishes it there once verified; any failure removes the staged
    /// file. One that another runner sharing the cache published first is
    /// the one installed.
    async fn download(
        &self,
        artifact: &PackageArtifact,
        destination: &Path,
        package: ForPackage<'_>,
        installing: &Installing,
        cancel: &CancellationToken,
    ) -> Result<(), RuntimeError> {
        let mut staged = Staged::new(
            destination,
            Publication {
                mode: Mode::CreateNew,
                permissions: Permissions::Executable,
                durable: true,
            },
        )
        .await?;
        self.fetch(artifact, staged.file(), package, installing, cancel)
            .await?;
        match staged.publish().await {
            Err(demi_shared_artifacts::Error::Io(error))
                if error.kind() == io::ErrorKind::AlreadyExists =>
            {
                cached(destination, artifact).await?;
                Ok(())
            }
            published => Ok(published?),
        }
    }

    /// Writes the verified bytes of `artifact`, from where the backend says
    /// it is, into `output`, reporting them to `installing`.
    async fn fetch(
        &self,
        artifact: &PackageArtifact,
        output: &mut (impl AsyncWrite + Unpin),
        package: ForPackage<'_>,
        installing: &Installing,
        cancel: &CancellationToken,
    ) -> Result<(), RuntimeError> {
        let expected = digest_of(artifact);
        // A URL that expired on the way is asked for once more.
        let mut refreshed = false;
        loop {
            let mut written = 0u64;
            let mut output = tokio_util::io::InspectWriter::new(&mut *output, |bytes: &[u8]| {
                written += bytes.len() as u64;
                installing.downloaded(written);
            });
            match package.resolver.resolve(artifact, cancel).await? {
                ArtifactSource::Local(path) => {
                    let mut input = tokio::fs::File::open(path).await?;
                    demi_shared_artifacts::copy(&mut input, &expected, &mut output, cancel).await?;
                }
                ArtifactSource::Url { url, expires_at } => {
                    let expired = || expires_at.is_some_and(|expires| expires <= SystemTime::now());
                    if expired() {
                        if refreshed {
                            return Err(RuntimeError::Location(
                                "the backend returned an expired URL".into(),
                            ));
                        }
                        refreshed = true;
                        continue;
                    }
                    let downloaded = demi_shared_artifacts::download(
                        &self.http,
                        &url,
                        &expected,
                        &mut output,
                        cancel,
                    )
                    .await;
                    match downloaded {
                        Err(demi_shared_artifacts::Error::Rejected { status: 403 })
                            if !refreshed && expired() =>
                        {
                            refreshed = true;
                            continue;
                        }
                        downloaded => downloaded?,
                    }
                }
            }
            return Ok(());
        }
    }
}

/// An artifact the image may hold: an executable of a size and SHA-256, or
/// a resource's archive unpacked.
#[derive(Clone, Copy)]
enum ImageCopy<'a> {
    Executable(&'a Digest),
    Resource(&'a Archive),
}

/// The size and SHA-256 `artifact` must have.
fn digest_of(artifact: &PackageArtifact) -> Digest {
    Digest {
        size: artifact.size,
        sha256: artifact.sha256.clone(),
    }
}

/// The cached executable at `destination`, when there is one. The entry was
/// verified as it was published, and only publication writes the cache, so
/// a hit is not read again: every service start would otherwise hash the
/// whole executable. Damage its metadata shows fails the install; it is not
/// repaired.
async fn cached(
    destination: &Path,
    artifact: &PackageArtifact,
) -> Result<Option<PathBuf>, RuntimeError> {
    let metadata = match tokio::fs::symlink_metadata(destination).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() {
        return Err(demi_shared_artifacts::Error::Digest.into());
    }
    if metadata.len() != artifact.size {
        return Err(demi_shared_artifacts::Error::Size {
            declared: artifact.size,
            actual: metadata.len(),
        }
        .into());
    }
    Ok(Some(destination.to_owned()))
}

/// Runs `attempt` until it does not fail for want of an open file, waiting
/// for one between attempts (`runner.md` § Load). A download has no
/// overall deadline: its client fails a connection that makes no progress.
async fn attempts<T, F>(
    cancel: &CancellationToken,
    mut attempt: impl FnMut() -> F,
) -> Result<T, RuntimeError>
where
    F: Future<Output = Result<T, RuntimeError>>,
{
    let mut backoff = demi_command_sdk::descriptors::Backoff::default();
    loop {
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(RuntimeError::Cancelled),
            result = attempt() => result,
        };
        match result {
            Err(error) if out_of_files(&error) => tokio::select! {
                _ = cancel.cancelled() => return Err(error),
                _ = backoff.wait() => {}
            },
            result => return result,
        }
    }
}

/// The image's installation of a resource's `archive` in `directory`,
/// checked whole: the receipt and the entry's SHA-256.
async fn image_resource(
    directory: &Path,
    archive: &Archive,
    cancel: &CancellationToken,
) -> Result<PathBuf, RuntimeError> {
    demi_shared_artifacts::installed(directory, archive, cancel)
        .await?
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound).into())
}

/// The image's copy in `directory`: the directory's one entry, a regular
/// file whose size and SHA-256 are the `expected` ones. A descriptor names
/// no file, so the copy is whatever the directory holds.
async fn check(
    directory: &Path,
    expected: &Digest,
    cancel: &CancellationToken,
) -> Result<PathBuf, RuntimeError> {
    let mut entries = tokio::fs::read_dir(directory).await?;
    let first = entries.next_entry().await?;
    let second = entries.next_entry().await?;
    let (Some(entry), None) = (first, second) else {
        return Err(io::Error::other("the directory does not hold exactly one file").into());
    };
    if !entry.file_type().await?.is_file() {
        return Err(io::Error::other("the directory's entry is not a regular file").into());
    }
    let executable = entry.path();
    let mut input = tokio::fs::File::open(&executable).await?;
    // Checked as a download is, with nowhere to write the bytes to.
    demi_shared_artifacts::copy(&mut input, expected, &mut tokio::io::sink(), cancel).await?;
    Ok(executable)
}

/// Whether the attempt failed for want of an open file.
fn out_of_files(error: &RuntimeError) -> bool {
    match error {
        RuntimeError::Io(error)
        | RuntimeError::Artifact(demi_shared_artifacts::Error::Io(error)) => {
            demi_command_sdk::descriptors::exhausted(error)
        }
        _ => false,
    }
}
