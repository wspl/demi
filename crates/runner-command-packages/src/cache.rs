//! The verified artifact cache (`native-runtime.md` § Install artifacts,
//! § The cache): a `file` as one file per digest, published only once its
//! size and SHA-256 match what the runner was given, and an `archive`
//! unpacked into a directory per digest with a receipt; both reused unread
//! from then on. Beside each entry a record names its line and version, so
//! the cache answers which artifacts of a line it has and removes a line's
//! older ones once a newer one is installed, except those a running service
//! holds. Before it downloads an artifact, it takes the copy the Host's image
//! preinstalled (§ Preinstalled artifacts), which it checks the same way the
//! first time the process needs it. Runners that share the cache publish one
//! copy of each artifact.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::SystemTime,
};

use demi_command_protocol::{ArtifactForm, InstalledArtifact, PackageArtifact};
use demi_shared_artifacts::{
    Archive, ArchiveInstall, Digest, Mode, Permissions, Publication, Staged,
};
use serde::{Deserialize, Serialize};
use demi_runner_protocol::wire::HostArtifact;
use tokio::io::AsyncWrite;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use demi_runner_process::private_files::chmod;

use crate::installs::{Installing, Installs};
use crate::{ArtifactResolver, ArtifactSource, RuntimeError};

/// What this process found when it checked the image's copy of each digest:
/// the file or the archive's entry, or none when the copy failed its check.
type Checked = HashMap<String, Option<PathBuf>>;

/// The suffix of an entry's line record, beside the entry.
const LINE: &str = ".line.json";

pub struct ArtifactCache {
    root: PathBuf,
    /// Where the Host's image preinstalls command artifacts, each in a
    /// directory named by its SHA-256. Nothing is there on a paired device.
    image: Option<PathBuf>,
    checked: Mutex<Checked>,
    http: reqwest::Client,
    installs: Installs,
    holds: Holds,
    /// What the cache holds, which the connection reports to the backend
    /// (`native-runtime.md` § Installed artifacts).
    contents: watch::Sender<Vec<HostArtifact>>,
}

/// An artifact to install: its line (its package and name) and version, its
/// bytes' size and SHA-256, and its form.
pub struct Wanted<'a> {
    pub package: &'a str,
    pub name: &'a str,
    pub version: &'a str,
    pub artifact: PackageArtifact,
    pub form: &'a ArtifactForm,
}

/// What the cache records of an entry, beside it: its line, its version,
/// the path of its file or entry, and when this cache installed it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LineRecord {
    package: String,
    name: String,
    version: String,
    path: PathBuf,
    installed_at: u64,
}

/// The digests running services hold, which no line's removal takes.
/// Cloning shares the counts.
#[derive(Clone, Default)]
pub struct Holds(Arc<Mutex<HashMap<String, usize>>>);

/// One hold of a digest, released when dropped.
pub struct Hold {
    holds: Holds,
    sha256: String,
}

impl Holds {
    /// Holds `sha256` until the returned hold is dropped.
    pub fn hold(&self, sha256: &str) -> Hold {
        *self.counts().entry(sha256.to_owned()).or_default() += 1;
        Hold {
            holds: self.clone(),
            sha256: sha256.to_owned(),
        }
    }

    fn held(&self, sha256: &str) -> bool {
        self.counts().contains_key(sha256)
    }

    fn counts(&self) -> MutexGuard<'_, HashMap<String, usize>> {
        self.0.lock().expect("the holds are intact")
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut counts = self.holds.counts();
        if let Some(count) = counts.get_mut(&self.sha256) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.sha256);
            }
        }
    }
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
        let cache = Self {
            root,
            image,
            checked: Mutex::default(),
            // The runner was given each artifact's digest over its
            // authenticated connection, so a backend on plain HTTP may serve
            // its artifacts itself.
            http: demi_shared_artifacts::client_allowing_http()?,
            installs,
            holds: Holds::default(),
            contents: watch::Sender::new(Vec::new()),
        };
        cache.publish_contents().await;
        Ok(cache)
    }

    /// What the cache holds now, and each later list: every artifact it
    /// recorded, by package, line and version.
    pub fn contents(&self) -> watch::Receiver<Vec<HostArtifact>> {
        self.contents.subscribe()
    }

    /// The digests running services hold.
    pub fn holds(&self) -> &Holds {
        &self.holds
    }

    /// The path of `wanted`'s file or of its archive's entry: the cached
    /// one, else the image's copy that matches it, else one downloaded into
    /// the cache. Once it is there, the cache records it in its line and
    /// removes the line's other artifacts that no running service holds. A
    /// cached entry whose metadata shows damage fails the install.
    pub async fn install(
        &self,
        wanted: &Wanted<'_>,
        resolver: &dyn ArtifactResolver,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, RuntimeError> {
        let path = match wanted.form {
            ArtifactForm::File => self.install_file(wanted, resolver, cancel).await?,
            ArtifactForm::Archive { entry } => {
                self.install_archive(wanted, entry, resolver, cancel)
                    .await?
            }
        };
        self.record(wanted, &path).await?;
        self.remove_older(wanted).await;
        self.publish_contents().await;
        Ok(path)
    }

    /// Publishes what the cache's records name, when it changed. A cache
    /// that cannot be listed keeps its last list, which the next install
    /// publishes again.
    async fn publish_contents(&self) {
        let records = match self.records().await {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!("the artifact cache could not be listed: {error}");
                return;
            }
        };
        let mut contents: Vec<HostArtifact> = records
            .into_iter()
            .map(|(_, record)| HostArtifact {
                package: record.package,
                name: record.name,
                version: record.version,
            })
            .collect();
        contents.sort();
        contents.dedup();
        self.contents.send_if_modified(|published| {
            if *published == contents {
                return false;
            }
            *published = contents;
            true
        });
    }

    /// The artifacts of the line `name` of `package` this cache installed,
    /// from a download or the image, the newest install first.
    pub async fn installed(
        &self,
        package: &str,
        name: &str,
    ) -> Result<Vec<InstalledArtifact>, RuntimeError> {
        let mut found: Vec<(u64, InstalledArtifact)> = self
            .records()
            .await?
            .into_iter()
            .filter(|(_, record)| record.package == package && record.name == name)
            .map(|(sha256, record)| {
                let installed = InstalledArtifact {
                    version: record.version,
                    sha256,
                    path: record.path.to_string_lossy().into_owned(),
                };
                (record.installed_at, installed)
            })
            .collect();
        found.sort_by(|(left, _), (right, _)| right.cmp(left));
        Ok(found.into_iter().map(|(_, installed)| installed).collect())
    }

    async fn install_file(
        &self,
        wanted: &Wanted<'_>,
        resolver: &dyn ArtifactResolver,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, RuntimeError> {
        let destination = self.root.join(&wanted.artifact.sha256);
        if let Some(cached) = cached(&destination, &wanted.artifact).await? {
            return Ok(cached);
        }
        let expected = digest_of(&wanted.artifact);
        attempts(cancel, || async {
            if let Some(file) = self
                .preinstalled(ImageCopy::File(&expected), cancel)
                .await?
            {
                return Ok(file);
            }
            let installing = self.start(wanted);
            self.download(wanted, &destination, resolver, &installing, cancel)
                .await?;
            Ok(destination.clone())
        })
        .await
    }

    async fn install_archive(
        &self,
        wanted: &Wanted<'_>,
        entry: &str,
        resolver: &dyn ArtifactResolver,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, RuntimeError> {
        let archive = Archive {
            digest: digest_of(&wanted.artifact),
            entry: entry.to_owned(),
        };
        let directory = self.root.join(&wanted.artifact.sha256);
        if let Some(entry) = demi_shared_artifacts::recorded(&directory, &archive).await? {
            return Ok(entry);
        }
        attempts(cancel, || async {
            if let Some(entry) = self
                .preinstalled(ImageCopy::Archive(&archive), cancel)
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
            let installing = self.start(wanted);
            let mut output = tokio::fs::File::create(unpacking.archive()).await?;
            self.fetch(&wanted.artifact, &mut output, resolver, &installing, cancel)
                .await?;
            drop(output);
            installing.unpacking();
            Ok(unpacking.finish(cancel).await?)
        })
        .await
    }

    /// Reports the download of `wanted` until the returned install drops.
    fn start(&self, wanted: &Wanted<'_>) -> Installing {
        self.installs.start(
            wanted.package,
            wanted.name,
            wanted.version,
            wanted.artifact.size,
        )
    }

    /// Records `wanted`, installed at `path`, in its line, unless the cache
    /// records it already.
    async fn record(&self, wanted: &Wanted<'_>, path: &Path) -> Result<(), RuntimeError> {
        let file = self.line_record(&wanted.artifact.sha256);
        if tokio::fs::try_exists(&file).await? {
            return Ok(());
        }
        let installed_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            });
        let record = LineRecord {
            package: wanted.package.to_owned(),
            name: wanted.name.to_owned(),
            version: wanted.version.to_owned(),
            path: path.to_owned(),
            installed_at,
        };
        let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;
        let publication = Publication {
            mode: Mode::Replace,
            permissions: Permissions::Private,
            durable: true,
        };
        demi_shared_artifacts::publish_bytes(&file, &bytes, publication).await?;
        Ok(())
    }

    /// Removes the other artifacts of `wanted`'s line that no running
    /// service holds. A removal that fails, as of a file in use on Windows,
    /// is left for a later install; an image's copy is only forgotten.
    async fn remove_older(&self, wanted: &Wanted<'_>) {
        let records = match self.records().await {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!("the artifact cache could not be listed: {error}");
                return;
            }
        };
        for (sha256, record) in records {
            let older = record.package == wanted.package
                && record.name == wanted.name
                && sha256 != wanted.artifact.sha256;
            if !older || self.holds.held(&sha256) {
                continue;
            }
            let entry = self.root.join(&sha256);
            let removed = match tokio::fs::symlink_metadata(&entry).await {
                Ok(metadata) if metadata.is_dir() => tokio::fs::remove_dir_all(&entry).await,
                Ok(_) => tokio::fs::remove_file(&entry).await,
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            };
            let removed = match removed {
                Ok(()) => tokio::fs::remove_file(self.line_record(&sha256)).await,
                Err(error) => Err(error),
            };
            if let Err(error) = removed {
                tracing::info!(
                    "{} {} stays in the artifact cache for now: {error}",
                    record.name,
                    record.version
                );
            }
        }
    }

    /// Every line record of the cache, by digest. A record that cannot be
    /// read is skipped: its entry is still a verified artifact, which a
    /// later install of it records again.
    async fn records(&self) -> Result<Vec<(String, LineRecord)>, RuntimeError> {
        let mut records = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.root).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            let Some(sha256) = name.to_str().and_then(|name| name.strip_suffix(LINE)) else {
                continue;
            };
            let read = tokio::fs::read(entry.path()).await;
            match read.map(|bytes| serde_json::from_slice::<LineRecord>(&bytes)) {
                Ok(Ok(record)) => records.push((sha256.to_owned(), record)),
                Ok(Err(error)) => tracing::warn!("{}: {error}", entry.path().display()),
                Err(error) => tracing::warn!("{}: {error}", entry.path().display()),
            }
        }
        Ok(records)
    }

    fn line_record(&self, sha256: &str) -> PathBuf {
        self.root.join(format!("{sha256}{LINE}"))
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
            ImageCopy::File(expected) => &expected.sha256,
            ImageCopy::Archive(archive) => &archive.digest.sha256,
        };
        if let Some(checked) = self.checked().get(sha256) {
            return Ok(checked.clone());
        }
        let directory = image.join(sha256);
        let found = match copy {
            ImageCopy::File(expected) => check(&directory, expected, cancel).await,
            ImageCopy::Archive(archive) => image_archive(&directory, archive, cancel).await,
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

    /// Fetches a `file` into a staged file beside `destination` and
    /// publishes it there once verified; any failure removes the staged
    /// file. One that another runner sharing the cache published first is
    /// the one installed.
    async fn download(
        &self,
        wanted: &Wanted<'_>,
        destination: &Path,
        resolver: &dyn ArtifactResolver,
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
        self.fetch(
            &wanted.artifact,
            staged.file(),
            resolver,
            installing,
            cancel,
        )
        .await?;
        match staged.publish().await {
            Err(demi_shared_artifacts::Error::Io(error))
                if error.kind() == io::ErrorKind::AlreadyExists =>
            {
                cached(destination, &wanted.artifact).await?;
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
        resolver: &dyn ArtifactResolver,
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
            match resolver.resolve(artifact, cancel).await? {
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

/// An artifact the image may hold: a file of a size and SHA-256, or an
/// archive unpacked.
#[derive(Clone, Copy)]
enum ImageCopy<'a> {
    File(&'a Digest),
    Archive(&'a Archive),
}

/// The size and SHA-256 `artifact` must have.
fn digest_of(artifact: &PackageArtifact) -> Digest {
    Digest {
        size: artifact.size,
        sha256: artifact.sha256.clone(),
    }
}

/// The cached file at `destination`, when there is one. The entry was
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

/// The image's installation of `archive` in `directory`, checked whole: the
/// receipt and the entry's SHA-256.
async fn image_archive(
    directory: &Path,
    archive: &Archive,
    cancel: &CancellationToken,
) -> Result<PathBuf, RuntimeError> {
    demi_shared_artifacts::installed(directory, archive, cancel)
        .await?
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound).into())
}

/// The image's copy in `directory`: the directory's one entry, a regular
/// file whose size and SHA-256 are the `expected` ones. A request names no
/// file, so the copy is whatever the directory holds.
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
