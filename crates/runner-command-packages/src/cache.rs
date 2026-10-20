//! The verified artifact cache (`native-runtime.md` § Install artifacts,
//! § The cache): a `file` as one file per digest, published only once its
//! size and SHA-256 match what the runner was given, and an `archive`
//! unpacked into a directory per digest with a receipt; both reused unread
//! from then on. Beside each entry a record names its line and version, so
//! the cache answers which artifacts of a line it has and removes a line's
//! older ones once a newer one is installed, except those a running service
//! holds. An install reports how its download goes, at each tenth of the
//! artifact's size and as it unpacks an archive, to whoever follows it
//! (§ Install artifacts). Before it downloads an artifact, it takes the copy the Host's image
//! preinstalled (§ Preinstalled artifacts), which it checks the same way the
//! first time the process needs it. Concurrent installs of one digest in
//! this process share one download, which lives while any of them waits
//! for it; runners that share the cache publish one copy of each artifact.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::SystemTime,
};

use demi_command_protocol::{ArtifactForm, ArtifactProgress, InstalledArtifact, PackageArtifact};
use demi_shared_artifacts::{
    Archive, ArchiveInstall, CONTENT_CODING, Digest, Download, Mode, Permissions, Publication,
    Staged,
};
use serde::{Deserialize, Serialize};
use demi_runner_process::backend::Backend;
use demi_runner_protocol::wire::HostArtifact;
use tokio::io::AsyncWrite;
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared, WeakShared};
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;

use demi_runner_process::private_files::chmod;

use crate::{ArtifactResolver, ArtifactSource, RuntimeError};

/// What this process found when it checked the image's copy of each digest:
/// the file or the archive's entry, or none when the copy failed its check.
type Checked = HashMap<String, Option<PathBuf>>;

/// How many redirects an artifact's download follows, as GitHub's release
/// downloads need (`native-runtime.md` § Install artifacts).
const REDIRECTS: usize = 10;

/// The suffix of an entry's line record, beside the entry.
const LINE: &str = ".line.json";

/// The prefix of a download's staging file, `.download-<sha256>`, which
/// keeps its bytes for a resume, beside its coding record, the line it is
/// of and its lock (`native-runtime.md` § Downloads).
const STAGING: &str = ".download-";

/// The suffix of the record of a staging file's line, beside it.
const STAGING_LINE: &str = ".of.json";

/// A line: its package and name.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Line {
    package: String,
    name: String,
}

/// Where an install reports how its download goes.
pub type Progress<'a> = &'a (dyn Fn(ArtifactProgress) + Send + Sync);

/// The progress of an install nobody follows, such as a program's own
/// executable, which installs before the command that needs it starts.
pub const SILENT: Progress<'static> = &|_| {};

/// An install of one digest that its callers share, from its fetch to its
/// publication.
type Install = BoxFuture<'static, Result<PathBuf, RuntimeError>>;

/// A shared install's progress, which each caller forwards to its own.
type Report = Arc<dyn Fn(ArtifactProgress) + Send + Sync>;

/// How many progress events a caller may fall behind: an attempt reports
/// eleven at most, ten tenths and its unpacking.
const PROGRESS_BACKLOG: usize = 64;

/// An install as its callers share it, whose error each of them reports.
type Sharing = BoxFuture<'static, Result<PathBuf, Arc<RuntimeError>>>;

/// An install in progress. The table holds it weakly, so the last caller
/// that leaves drops it.
struct InFlight {
    install: WeakShared<Sharing>,
    callers: Arc<Mutex<Callers>>,
    progress: Arc<Mutex<Reported>>,
}

impl InFlight {
    /// Whether the install still runs; one that ended, as one that
    /// failed, is not joined but started again.
    fn running(&self) -> Option<Shared<Sharing>> {
        self.install
            .upgrade()
            .filter(|install| install.peek().is_none())
    }
}

/// The callers waiting for an install, in the order they joined, each with
/// the resolver that locates the artifact for its work.
#[derive(Default)]
struct Callers {
    next: u64,
    waiting: Vec<Caller>,
}

struct Caller {
    id: u64,
    resolver: Arc<dyn ArtifactResolver>,
    /// Cancelled when the caller is cancelled or leaves.
    left: CancellationToken,
}

impl Callers {
    fn lock(callers: &Mutex<Self>) -> MutexGuard<'_, Self> {
        callers.lock().expect("the waiting callers are intact")
    }

    fn add(&mut self, resolver: Arc<dyn ArtifactResolver>, cancel: &CancellationToken) -> u64 {
        let id = self.next;
        self.next += 1;
        self.waiting.push(Caller {
            id,
            resolver,
            left: cancel.child_token(),
        });
        id
    }

    fn remove(&mut self, id: u64) {
        if let Some(index) = self.waiting.iter().position(|caller| caller.id == id) {
            self.waiting.remove(index).left.cancel();
        }
    }
}

/// How far a shared install got, which a caller that joins reports first,
/// and the channel of what it reports later. One lock orders both, so a
/// joiner misses no event and hears none twice.
struct Reported {
    last: Option<ArtifactProgress>,
    sender: broadcast::Sender<ArtifactProgress>,
}

impl Reported {
    fn lock(progress: &Mutex<Self>) -> MutexGuard<'_, Self> {
        progress.lock().expect("the install's progress is intact")
    }
}

/// Locates a shared download's artifact by the resolver of the first
/// caller still waiting (`native-runtime.md` § Install artifacts): the one
/// that started the download may have left by the time an expired URL or a
/// retry asks again, and a caller that leaves while its resolver answers
/// hands the question to the next.
struct Waiting(Arc<Mutex<Callers>>);

impl ArtifactResolver for Waiting {
    fn resolve<'a>(
        &'a self,
        artifact: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            loop {
                let first = Callers::lock(&self.0)
                    .waiting
                    .iter()
                    .find(|caller| !caller.left.is_cancelled())
                    .map(|caller| (caller.resolver.clone(), caller.left.clone()));
                // With nobody waiting, the install is about to be dropped.
                let Some((resolver, left)) = first else {
                    return Err(RuntimeError::Cancelled);
                };
                let resolved = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(RuntimeError::Cancelled),
                    _ = left.cancelled() => continue,
                    resolved = resolver.resolve(artifact, &left) => resolved,
                };
                match resolved {
                    Err(_) if left.is_cancelled() => continue,
                    resolved => return resolved,
                }
            }
        })
    }
}

/// One caller's share of an install. Dropping it, as the caller ends or is
/// dropped, takes the caller off the install's waiting callers, and the
/// install off the table once nobody waits for it or it ended.
struct Joined<'a> {
    installs: &'a Mutex<HashMap<String, InFlight>>,
    sha256: &'a str,
    callers: Arc<Mutex<Callers>>,
    id: u64,
    install: Option<Shared<Sharing>>,
}

impl Drop for Joined<'_> {
    fn drop(&mut self) {
        // The share goes first, so the table sees whether another remains.
        drop(self.install.take());
        Callers::lock(&self.callers).remove(self.id);
        let mut installs = self.installs.lock().expect("the installs in flight are intact");
        if installs
            .get(self.sha256)
            .is_some_and(|in_flight| in_flight.running().is_none())
        {
            installs.remove(self.sha256);
        }
    }
}

pub struct ArtifactCache {
    root: PathBuf,
    /// Where the Host's image preinstalls command artifacts, each in a
    /// directory named by its SHA-256. Nothing is there on a paired device.
    image: Option<PathBuf>,
    checked: Mutex<Checked>,
    http: reqwest::Client,
    /// The backend that hands out the artifacts' URLs, with the client that
    /// reaches it, when that takes its own way, as a Cloud's socket does.
    backend: Option<(Backend, reqwest::Client)>,
    holds: Holds,
    /// The installs in progress, by digest, which concurrent callers share.
    installs: Mutex<HashMap<String, InFlight>>,
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

impl Wanted<'_> {
    fn line(&self) -> Line {
        Line {
            package: self.package.to_owned(),
            name: self.name.to_owned(),
        }
    }
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
    /// when given, before it downloads one.
    pub async fn new(
        root: PathBuf,
        image: Option<PathBuf>,
        backend: Option<Backend>,
    ) -> Result<Self, RuntimeError> {
        let backend = match backend {
            Some(backend) => {
                let http =
                    demi_shared_artifacts::download_client_through(|builder| backend.http(builder))?;
                Some((backend, http))
            }
            None => None,
        };
        tokio::fs::create_dir_all(&root).await?;
        chmod(&root, 0o700).await?;
        let cache = Self {
            root,
            image,
            checked: Mutex::default(),
            http: demi_shared_artifacts::download_client_allowing_http()?,
            backend,
            holds: Holds::default(),
            installs: Mutex::default(),
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
    /// cached entry whose metadata shows damage fails the install. A
    /// download reports to `progress` at each tenth of the artifact's size,
    /// and an archive once more as it is unpacked. Concurrent installs of
    /// one digest share one download, located by the resolver of the
    /// install that started it.
    pub async fn install(
        self: &Arc<Self>,
        wanted: &Wanted<'_>,
        resolver: Arc<dyn ArtifactResolver>,
        progress: Progress<'_>,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, RuntimeError> {
        let path = match wanted.form {
            ArtifactForm::File => {
                self.install_file(wanted, resolver, progress, cancel)
                    .await?
            }
            ArtifactForm::Archive { entry } => {
                self.install_archive(wanted, entry, resolver, progress, cancel)
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
        self: &Arc<Self>,
        wanted: &Wanted<'_>,
        resolver: Arc<dyn ArtifactResolver>,
        progress: Progress<'_>,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, RuntimeError> {
        let cache = self.clone();
        let artifact = wanted.artifact.clone();
        let line = wanted.line();
        let sha256 = &wanted.artifact.sha256;
        self.shared(sha256, resolver, progress, cancel, move |report, resolver, cancel| {
            Box::pin(async move {
                let destination = cache.root.join(&artifact.sha256);
                if let Some(cached) = cached(&destination, &artifact).await? {
                    return Ok(cached);
                }
                let expected = digest_of(&artifact);
                attempts(&cancel, || async {
                    if let Some(file) = cache
                        .preinstalled(ImageCopy::File(&expected), &cancel)
                        .await?
                    {
                        return Ok(file);
                    }
                    cache
                        .download(&line, &artifact, &destination, resolver.as_ref(), &*report, &cancel)
                        .await?;
                    Ok(destination.clone())
                })
                .await
            })
        })
        .await
    }

    async fn install_archive(
        self: &Arc<Self>,
        wanted: &Wanted<'_>,
        entry: &str,
        resolver: Arc<dyn ArtifactResolver>,
        progress: Progress<'_>,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, RuntimeError> {
        let cache = self.clone();
        let artifact = wanted.artifact.clone();
        let line = wanted.line();
        let archive = Archive {
            digest: digest_of(&wanted.artifact),
            entry: entry.to_owned(),
        };
        let sha256 = &wanted.artifact.sha256;
        self.shared(sha256, resolver, progress, cancel, move |report, resolver, cancel| {
            Box::pin(async move {
                let directory = cache.root.join(&artifact.sha256);
                if let Some(entry) = demi_shared_artifacts::recorded(&directory, &archive).await? {
                    return Ok(entry);
                }
                attempts(&cancel, || async {
                    if let Some(entry) = cache
                        .preinstalled(ImageCopy::Archive(&archive), &cancel)
                        .await?
                    {
                        return Ok(entry);
                    }
                    let unpacking =
                        match demi_shared_artifacts::install_archive(&cache.root, &archive, &cancel)
                            .await?
                        {
                            // Another runner sharing the cache installed it meanwhile.
                            ArchiveInstall::Installed(entry) => return Ok(entry),
                            ArchiveInstall::Unpack(unpacking) => unpacking,
                        };
                    let mut output = tokio::fs::File::create(unpacking.archive()).await?;
                    cache
                        .fetch(&line, &artifact, &mut output, resolver.as_ref(), &*report, &cancel)
                        .await?;
                    drop(output);
                    (*report)(ArtifactProgress::Unpack);
                    Ok(unpacking.finish(&cancel).await?)
                })
                .await
            })
        })
        .await
    }

    /// Waits for the install of `sha256` in progress, or starts it with
    /// `start` when none is (`native-runtime.md` § Install artifacts:
    /// concurrent callers share one download per digest). The install
    /// lives while any caller waits for it: one caller's cancellation only
    /// leaves it, and the last one's drops it, which cancels the token it
    /// was started with and so removes what it staged. It locates the
    /// artifact by the resolver of a caller still waiting. A caller that
    /// joins reports where the install already is, then what it reports
    /// later. The caller joins before its first await, so a caller polled
    /// once shares the install.
    async fn shared(
        &self,
        sha256: &str,
        resolver: Arc<dyn ArtifactResolver>,
        progress: Progress<'_>,
        cancel: &CancellationToken,
        start: impl FnOnce(Report, Arc<dyn ArtifactResolver>, CancellationToken) -> Install,
    ) -> Result<PathBuf, RuntimeError> {
        let (mut joined, mut events, last) = {
            let mut installs = self.installs();
            let running = installs
                .get(sha256)
                .and_then(|in_flight| Some((in_flight.running()?, in_flight)));
            let (install, callers, reported) = match running {
                Some((install, in_flight)) => {
                    (install, in_flight.callers.clone(), in_flight.progress.clone())
                }
                None => {
                    let (sender, _) = broadcast::channel(PROGRESS_BACKLOG);
                    let reported = Arc::new(Mutex::new(Reported { last: None, sender }));
                    let report: Report = Arc::new({
                        let reported = reported.clone();
                        move |event| {
                            let mut reported = Reported::lock(&reported);
                            reported.last = Some(event);
                            // Nobody follows the install when no caller hears it.
                            let _heard = reported.sender.send(event);
                        }
                    });
                    let callers = Arc::new(Mutex::new(Callers::default()));
                    let token = CancellationToken::new();
                    let guard = token.clone().drop_guard();
                    let started = start(report, Arc::new(Waiting(callers.clone())), token);
                    let install = async move {
                        let _cancel_when_dropped = guard;
                        started.await.map_err(Arc::new)
                    }
                    .boxed()
                    .shared();
                    let weak = install
                        .downgrade()
                        .expect("an install that was never polled has not finished");
                    installs.insert(
                        sha256.to_owned(),
                        InFlight {
                            install: weak,
                            callers: callers.clone(),
                            progress: reported.clone(),
                        },
                    );
                    (install, callers, reported)
                }
            };
            let id = Callers::lock(&callers).add(resolver, cancel);
            let (events, last) = {
                let reported = Reported::lock(&reported);
                (reported.sender.subscribe(), reported.last)
            };
            let joined = Joined {
                installs: &self.installs,
                sha256,
                callers,
                id,
                install: Some(install),
            };
            (joined, events, last)
        };
        if let Some(last) = last {
            progress(last);
        }
        let install = joined.install.as_mut().expect("the share is held until the caller ends");
        let result = loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(RuntimeError::Cancelled),
                Ok(event) = events.recv() => progress(event),
                result = &mut *install => break result,
            }
        };
        // What the install reported in the poll that finished it.
        while let Ok(event) = events.try_recv() {
            progress(event);
        }
        result.map_err(|error| Arc::try_unwrap(error).unwrap_or_else(RuntimeError::Shared))
    }

    fn installs(&self) -> MutexGuard<'_, HashMap<String, InFlight>> {
        self.installs.lock().expect("the installs in flight are intact")
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
    /// service holds, and the staging files of its other versions that no
    /// install of this cache is downloading (`native-runtime.md` §
    /// Downloads). A removal that fails, as of a file in use on Windows, is
    /// left for a later install; an image's copy is only forgotten.
    async fn remove_older(&self, wanted: &Wanted<'_>) {
        self.remove_older_staging(wanted).await;
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

    /// Removes the staging files of `wanted`'s line but its own that no
    /// install in progress downloads.
    async fn remove_older_staging(&self, wanted: &Wanted<'_>) {
        let line = wanted.line();
        let mut entries = match tokio::fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!("the artifact cache could not be listed: {error}");
                return;
            }
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let Some(sha256) = name
                .to_str()
                .and_then(|name| name.strip_prefix(STAGING)?.strip_suffix(STAGING_LINE))
            else {
                continue;
            };
            if sha256 == wanted.artifact.sha256 || self.installs().contains_key(sha256) {
                continue;
            }
            // A record that cannot be read names no line; its staging file
            // stays for its own install.
            let of = tokio::fs::read(entry.path()).await.ok();
            if of.and_then(|bytes| serde_json::from_slice::<Line>(&bytes).ok()) != Some(line.clone()) {
                continue;
            }
            demi_shared_artifacts::discard(&self.root.join(format!("{STAGING}{sha256}"))).await;
            // Each goes when it can; one left is pruned at a later install.
            let _removed = tokio::fs::remove_file(self.root.join(format!("{STAGING}{sha256}.lock"))).await;
            let _removed = tokio::fs::remove_file(entry.path()).await;
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
        line: &Line,
        artifact: &PackageArtifact,
        destination: &Path,
        resolver: &dyn ArtifactResolver,
        progress: Progress<'_>,
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
        self.fetch(line, artifact, staged.file(), resolver, progress, cancel)
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
    /// it is, into `output`, reporting to `progress` each time they pass
    /// another tenth of its size. A URL's bytes arrive as they are served,
    /// in their content coding, into a staging file beside the cache that
    /// keeps them across broken connections and attempts, and are decoded
    /// and checked once complete (`native-runtime.md` § Install
    /// artifacts); bytes that fail the check are removed.
    async fn fetch(
        &self,
        line: &Line,
        artifact: &PackageArtifact,
        output: &mut (impl AsyncWrite + Unpin),
        resolver: &dyn ArtifactResolver,
        progress: Progress<'_>,
        cancel: &CancellationToken,
    ) -> Result<(), RuntimeError> {
        let expected = digest_of(artifact);
        // Each tenth of `total` that `done` passes is reported once.
        let mut reported = 0;
        let mut report = |done: u64, total: u64| {
            let tenth = done.saturating_mul(10) / total.max(1);
            if tenth > reported {
                reported = tenth;
                progress(ArtifactProgress::Download {
                    done: done.min(total),
                    total,
                });
            }
        };
        let (mut url, mut expires_at) = match located(resolver, artifact, cancel).await? {
            ArtifactSource::Local(path) => {
                let mut written = 0u64;
                let mut output = tokio_util::io::InspectWriter::new(&mut *output, |bytes: &[u8]| {
                    written += bytes.len() as u64;
                    report(written, artifact.size);
                });
                let mut input = tokio::fs::File::open(path).await?;
                demi_shared_artifacts::copy(&mut input, &expected, &mut output, cancel).await?;
                return Ok(());
            }
            ArtifactSource::Url { url, expires_at } => (url, expires_at),
        };
        let sha256 = &artifact.sha256;
        // Runners sharing the cache keep one staging file for each artifact.
        let _lock = demi_shared_artifacts::InstallLock::acquire(
            &self.root.join(format!("{STAGING}{sha256}.lock")),
            cancel,
        )
        .await?;
        let staging = self.root.join(format!("{STAGING}{sha256}"));
        // The line the staging file is of, so a newer version of the line
        // prunes it.
        let bytes = serde_json::to_vec(line).map_err(io::Error::other)?;
        tokio::fs::write(self.root.join(format!("{STAGING}{sha256}{STAGING_LINE}")), bytes).await?;
        // Stored bytes are asked for as they are, in the coding the check
        // decodes, so a range counts them.
        let headers = |_: &reqwest::Url| {
            reqwest::header::HeaderMap::from_iter([(
                reqwest::header::ACCEPT_ENCODING,
                reqwest::header::HeaderValue::from_static(CONTENT_CODING),
            )])
        };
        let received = loop {
            // The backend's own store is fetched the way the runner
            // reaches the backend, which no redirect may leave.
            let local = self.backend.as_ref().and_then(|(backend, http)| {
                let local = backend.local(&reqwest::Url::parse(&url).ok()?)?;
                Some((http, local.to_string()))
            });
            let redirects = if local.is_some() { 0 } else { REDIRECTS };
            let (http, location) = local.unwrap_or((&self.http, url.clone()));
            // The runner was given each artifact's digest over its
            // authenticated connection, so plain HTTP cannot change what it
            // keeps, nor can the host a redirect names.
            // No form of the artifact's bytes is larger than this, so more
            // fail as they arrive.
            let request = Download {
                limit: demi_shared_artifacts::encoded_bound(artifact.size),
                redirects,
                headers: &headers,
                // A signed URL that expires is asked for again below, and
                // the download continues from the renewed one.
                renewable: expires_at.is_some(),
                ..Download::new(&location)
            };
            let downloaded = demi_shared_artifacts::download(
                http,
                request,
                &staging,
                cancel,
                |held, total| report(held, total.unwrap_or(artifact.size)),
            )
            .await;
            match downloaded {
                // A URL that expired while the bytes arrived is asked for
                // again, and the download resumes from the new one.
                Err(demi_shared_artifacts::Error::Rejected { status: 403 })
                    if expires_at.is_some_and(|expires| expires <= SystemTime::now()) =>
                {
                    let ArtifactSource::Url {
                        url: renewed,
                        expires_at: renewed_expiry,
                    } = located(resolver, artifact, cancel).await?
                    else {
                        return Err(RuntimeError::Location(
                            "the backend moved the artifact from a URL to a local path".into(),
                        ));
                    };
                    url = renewed;
                    expires_at = renewed_expiry;
                }
                // A refusal of an address not renewed ends the staging file
                // the download kept for a renewal.
                Err(refused @ demi_shared_artifacts::Error::Rejected { .. }) => {
                    demi_shared_artifacts::discard(&staging).await;
                    return Err(refused.into());
                }
                downloaded => break downloaded?,
            }
        };
        received.verify(&staging, &expected, output, cancel).await?;
        // The staging file has gone with its check.
        let _removed =
            tokio::fs::remove_file(self.root.join(format!("{STAGING}{sha256}{STAGING_LINE}"))).await;
        Ok(())
    }
}

/// Where `resolver` says `artifact` is: a URL that has expired by the time
/// it arrives is asked for once more, and a second expired one fails.
async fn located(
    resolver: &dyn ArtifactResolver,
    artifact: &PackageArtifact,
    cancel: &CancellationToken,
) -> Result<ArtifactSource, RuntimeError> {
    let expired = |source: &ArtifactSource| {
        matches!(source, ArtifactSource::Url { expires_at: Some(expires), .. } if *expires <= SystemTime::now())
    };
    let source = resolver.resolve(artifact, cancel).await?;
    if !expired(&source) {
        return Ok(source);
    }
    let source = resolver.resolve(artifact, cancel).await?;
    if expired(&source) {
        return Err(RuntimeError::Location(
            "the backend returned an expired URL".into(),
        ));
    }
    Ok(source)
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
/// for one between attempts (`crates/command-sdk/src/descriptors.rs`). A download has no
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
