//! Execution contexts (`crates/command-protocol/src/invocation.rs`): the live
//! authority a job's declared commands run under, and the manifest the
//! connection installed. The connection owns its contexts; everything else
//! looks them up in the snapshot it publishes.

use crate::{connection::ConnectionHandle, job_media::JobMedia};
use demi_command_protocol::{CommandContext, EditContext, Viewable, host_target};
use demi_runner_command_packages::{ResidentHandle, ResidentLease};
use demi_runner_process::{
    command_client::{CONTEXT_ENV, ENDPOINT_ENV},
    private_files::{chmod, write_private},
};
use demi_runner_protocol::manifest::Manifest;
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub struct ExecutionContext {
    pub id: String,
    pub job_id: String,
    /// What the backend told the job's declared commands
    /// (`crates/command-protocol/src/invocation.rs`).
    pub command: CommandContext,
    pub manifest: Arc<Manifest>,
    pub cancel: CancellationToken,
    /// Where the job records the files its commands change.
    pub edits: EditContext,
    /// The connection the job runs under, for its callbacks and artifact
    /// locations.
    pub connection: ConnectionHandle,
    /// Where the media the job's commands return go.
    pub media: Arc<JobMedia>,
    /// The media types the job may show the model, as the backend gave
    /// them; none for a job whose media reach no model.
    pub viewable: Option<Viewable>,
    aliases: Aliases,
    /// How many leases of the context are held (`ContextLease`).
    leased: AtomicUsize,
    /// Whether the job has ended and only what it left running holds the
    /// context.
    ended: AtomicBool,
}

impl ExecutionContext {
    /// A context for `job_id` with a private directory holding one alias of
    /// the runner per root command. It is live once its connection has
    /// registered it.
    pub async fn create(
        job_id: String,
        command: CommandContext,
        manifest: Arc<Manifest>,
        edits: EditContext,
        media: Arc<JobMedia>,
        viewable: Option<Viewable>,
        connection: ConnectionHandle,
        paths: &ContextPaths,
    ) -> io::Result<Self> {
        let aliases = aliases(
            paths.directory.clone(),
            paths.executable.clone(),
            manifest.roots.keys().cloned().collect(),
        )
        .await?;
        Ok(Self {
            id: uuid::Uuid::new_v4().simple().to_string(),
            job_id,
            command,
            manifest,
            cancel: CancellationToken::new(),
            edits,
            connection,
            media,
            viewable,
            aliases,
            leased: AtomicUsize::new(0),
            ended: AtomicBool::new(false),
        })
    }

    /// The context a runner before this one made for `job_id`, whose id and
    /// alias directory the job's processes still name, taken over with the
    /// job (`crates/runner-shell/src/remote/mod.rs`).
    pub fn restore(
        id: String,
        job_id: String,
        command: CommandContext,
        manifest: Arc<Manifest>,
        edits: EditContext,
        media: Arc<JobMedia>,
        viewable: Option<Viewable>,
        connection: ConnectionHandle,
        aliases: PathBuf,
    ) -> Self {
        Self {
            id,
            job_id,
            command,
            manifest,
            cancel: CancellationToken::new(),
            edits,
            connection,
            media,
            viewable,
            aliases: Aliases::new(aliases),
            leased: AtomicUsize::new(0),
            ended: AtomicBool::new(false),
        }
    }

    /// The directory of the context's aliases, which the job's `PATH` names
    /// first.
    pub fn aliases(&self) -> &Path {
        &self.aliases.path
    }

    /// The runner leaves the context's job to the runner that starts next,
    /// which makes the context again: its alias directory stays.
    pub fn leave(&self) {
        self.aliases.kept.store(true, Ordering::SeqCst);
    }

    pub fn environment(
        &self,
        endpoint: &str,
        home: &str,
        path: Option<&str>,
    ) -> io::Result<BTreeMap<String, String>> {
        let mut paths = vec![self.aliases.path.clone()];
        if let Some(path) = path {
            paths.extend(std::env::split_paths(path));
        }
        let path = std::env::join_paths(paths).map_err(io::Error::other)?;
        let path = path
            .into_string()
            .map_err(|_| io::Error::other("command PATH is not UTF-8"))?;
        Ok(BTreeMap::from([
            (ENDPOINT_ENV.into(), endpoint.into()),
            (CONTEXT_ENV.into(), self.id.clone()),
            ("DEMI_HOME".into(), home.into()),
            ("PATH".into(), path),
        ]))
    }

    /// Whether something holds the context beyond the job's own end.
    fn leased(&self) -> bool {
        self.leased.load(Ordering::SeqCst) > 0
    }

    /// Whether the job has ended, and what it left running holds the context:
    /// the job's report of edits has gone, so that work records none.
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }

    /// Whether the context's manifest carries `digest` for this host.
    pub fn carries(&self, digest: &str) -> bool {
        self.manifest
            .packages
            .values()
            .any(|package| package.carries(host_target(), digest).is_some())
    }
}

/// A hold on a job's context for as long as anything the job started runs,
/// which is longer than the job when it left something running: that work still
/// runs the job's declared commands and finds its aliases. The context ends once
/// the job has ended and the last lease is dropped.
pub struct ContextLease {
    context: Arc<ExecutionContext>,
    runtime: tokio::runtime::Handle,
}

impl ContextLease {
    /// A lease on `context`, made inside the runtime that serves the connection.
    pub fn new(context: Arc<ExecutionContext>) -> Self {
        context.leased.fetch_add(1, Ordering::SeqCst);
        Self {
            context,
            runtime: tokio::runtime::Handle::current(),
        }
    }
}

impl Drop for ContextLease {
    fn drop(&mut self) {
        if self.context.leased.fetch_sub(1, Ordering::SeqCst) == 1 {
            let context = self.context.clone();
            self.runtime.spawn(async move {
                context.connection.context_released(context.job_id.clone()).await;
            });
        }
    }
}

/// Where contexts keep their alias directories and manifests, and what the
/// aliases run.
#[derive(Clone)]
pub struct ContextPaths {
    pub directory: PathBuf,
    pub executable: PathBuf,
}

impl ContextPaths {
    pub async fn new(directory: PathBuf, executable: PathBuf) -> io::Result<Self> {
        tokio::fs::create_dir_all(&directory).await?;
        chmod(&directory, 0o700).await?;
        Ok(Self {
            directory,
            executable,
        })
    }
}

/// The live contexts by id, as their connection last published them.
pub type ContextIndex = HashMap<String, Arc<ExecutionContext>>;

/// Looks contexts up in the snapshot the current connection publishes.
#[derive(Clone)]
pub struct Contexts(watch::Receiver<Arc<ContextIndex>>);

impl Contexts {
    pub fn new(index: watch::Receiver<Arc<ContextIndex>>) -> Self {
        Self(index)
    }

    pub fn get(&self, id: &str) -> io::Result<Arc<ExecutionContext>> {
        self.0.borrow().get(id).cloned().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "execution context is not live on this runner",
            )
        })
    }

    /// A live context whose manifest carries `digest`, which authorizes its
    /// download.
    pub fn carrying(&self, digest: &str) -> Option<Arc<ExecutionContext>> {
        self.0
            .borrow()
            .values()
            .find(|context| !context.cancel.is_cancelled() && context.carries(digest))
            .cloned()
    }
}

/// A connection's contexts. The connection owns the table and publishes each
/// change as a new snapshot; a context's resident leases end when it leaves.
pub struct ContextTable {
    index: watch::Sender<Arc<ContextIndex>>,
    leases: HashMap<String, PackageLeases>,
    /// The jobs that ended while something they left running held their
    /// context: it ends once that is gone (`ContextTable::released`).
    ending: HashSet<String>,
}

impl ContextTable {
    pub fn new(index: watch::Sender<Arc<ContextIndex>>) -> Self {
        index.send_replace(Arc::default());
        Self {
            index,
            leases: HashMap::new(),
            ending: HashSet::new(),
        }
    }

    /// Makes `context` live; a job has at most one.
    /// The runner leaves every context's job to the next runner: each keeps
    /// what its job's processes still name.
    pub fn leave(&self) {
        for context in self.index.borrow().values() {
            context.leave();
        }
    }

    pub fn insert(
        &mut self,
        context: Arc<ExecutionContext>,
        leases: PackageLeases,
    ) -> io::Result<()> {
        let mut entries = ContextIndex::clone(&self.index.borrow());
        if entries.values().any(|entry| entry.job_id == context.job_id) {
            return Err(io::Error::other("duplicate live execution owner"));
        }
        self.leases.insert(context.id.clone(), leases);
        entries.insert(context.id.clone(), context);
        self.index.send_replace(Arc::new(entries));
        Ok(())
    }

    /// Cancels the context of `job_id`; it stays until the job ends.
    pub fn cancel(&self, job_id: &str) {
        for context in self.index.borrow().values() {
            if context.job_id == job_id {
                context.cancel.cancel();
            }
        }
    }

    /// Ends the context of `job_id`, once its process and streams finished and
    /// nothing it left running holds it; whether it ended now.
    pub fn remove(&mut self, job_id: &str) -> bool {
        let mut entries = ContextIndex::clone(&self.index.borrow());
        let Some(id) = entries
            .values()
            .find(|context| context.job_id == job_id)
            .map(|context| context.id.clone())
        else {
            return false;
        };
        if entries[&id].leased() {
            entries[&id].ended.store(true, Ordering::SeqCst);
            self.ending.insert(job_id.to_owned());
            return false;
        }
        if let Some(context) = entries.remove(&id) {
            context.cancel.cancel();
        }
        self.leases.remove(&id);
        self.index.send_replace(Arc::new(entries));
        true
    }
}

impl ContextTable {
    /// The last lease on the context of `job_id` was dropped: it ends if its
    /// job has; whether it ended now.
    pub fn released(&mut self, job_id: &str) -> bool {
        self.ending.remove(job_id) && self.remove(job_id)
    }
}

impl Drop for ContextTable {
    /// A closed connection's contexts end with it.
    fn drop(&mut self) {
        for context in self.index.borrow().values() {
            context.cancel.cancel();
        }
        self.index.send_replace(Arc::default());
    }
}

/// The resident leases of a manifest's packages for this host, by package
/// id.
pub type PackageLeases = BTreeMap<String, ResidentLease>;

/// The manifests one connection installed, by hash, and those it is still
/// installing; they end with the connection (`crates/runner-jobs/src/commands/mod.rs`).
#[derive(Default)]
pub struct Manifests {
    installed: HashMap<String, Arc<Manifest>>,
    /// The installs under way, by number; each announces its manifest once
    /// kept, and one that fails ends without announcing.
    installing: BTreeMap<u64, watch::Receiver<Option<Arc<Manifest>>>>,
    installs: u64,
    /// For each package, the lease on the release that the latest manifest
    /// naming the package names, with that manifest's install number
    /// (`crates/runner-command-packages/src/registry.rs`): an older release's
    /// resident ends with the last job pinned to it.
    leases: HashMap<String, (u64, ResidentLease)>,
}

impl Manifests {
    /// Starts installing `value`: the install counts for the starts that
    /// arrive from now on, and the future it returns, with its number,
    /// goes to [`Manifests::installed`].
    pub fn install(
        &mut self,
        value: serde_json::Value,
        paths: ContextPaths,
        residents: ResidentHandle,
        reserved: BTreeSet<String>,
    ) -> impl Future<Output = (u64, io::Result<Installed>)> + Send + use<> {
        self.installs += 1;
        let number = self.installs;
        let (announce, installing) = watch::channel(None);
        self.installing.insert(number, installing);
        async move {
            let result = install(value, &paths, &residents, &reserved).await;
            if let Ok(installed) = &result {
                announce.send_replace(Some(installed.manifest.clone()));
            }
            (number, result)
        }
    }

    /// Keeps what the install `number` made, for the connection's life.
    /// Each package's lease moves to its release when no later install
    /// named the package; the lease it replaces ends after the new one
    /// counts, so a release both name stays resident.
    pub fn installed(&mut self, number: u64, result: io::Result<Installed>) -> io::Result<()> {
        self.installing.remove(&number);
        let Installed { manifest, leases } = result?;
        for (package, lease) in leases {
            let later = self
                .leases
                .get(&package)
                .is_some_and(|(latest, _)| *latest > number);
            if !later {
                self.leases.insert(package, (number, lease));
            }
        }
        self.installed.insert(manifest.hash.clone(), manifest);
        Ok(())
    }

    /// The manifest of `hash` for a job whose start arrives now: one
    /// installed, or one of the installs under way, which arrived first.
    pub fn pin(&self, hash: String) -> PinnedManifest {
        match self.installed.get(&hash) {
            Some(manifest) => PinnedManifest {
                hash,
                installed: Some(manifest.clone()),
                installing: Vec::new(),
            },
            None => PinnedManifest {
                hash,
                installed: None,
                installing: self.installing.values().cloned().collect(),
            },
        }
    }
}

/// The manifest a job's start names, as the connection had it when the
/// start arrived; later manifests leave it as it is.
pub struct PinnedManifest {
    hash: String,
    installed: Option<Arc<Manifest>>,
    installing: Vec<watch::Receiver<Option<Arc<Manifest>>>>,
}

impl PinnedManifest {
    /// The manifest, once installed; an error when the connection
    /// installed none of its hash before the start. It waits for the
    /// installs under way together, so one that stalls holds up only the
    /// jobs pinned to it.
    pub async fn manifest(self) -> io::Result<Arc<Manifest>> {
        let Self {
            hash,
            installed,
            installing,
        } = self;
        if let Some(manifest) = installed {
            return Ok(manifest);
        }
        let hash = &hash;
        let mut announced: FuturesUnordered<_> = installing
            .into_iter()
            .map(|mut installing| async move {
                // An install that failed ended without a manifest.
                let manifest = installing.wait_for(Option::is_some).await.ok()?.clone();
                manifest.filter(|manifest| manifest.hash == *hash)
            })
            .collect();
        while let Some(manifest) = announced.next().await {
            if let Some(manifest) = manifest {
                return Ok(manifest);
            }
        }
        Err(io::Error::other("job manifest is not installed"))
    }
}

/// A manifest the connection installed, with the leases that keep its
/// packages' residents alive (`crates/runner-command-packages/src/registry.rs`).
pub struct Installed {
    pub manifest: Arc<Manifest>,
    pub leases: PackageLeases,
}

/// Parses, checks and keeps `value` as the connection's manifest; a root
/// command may not take one of the `reserved` names, which the runner and
/// its shell answer to themselves.
pub async fn install(
    value: serde_json::Value,
    paths: &ContextPaths,
    residents: &ResidentHandle,
    reserved: &BTreeSet<String>,
) -> io::Result<Installed> {
    let manifest = tokio::task::spawn_blocking(move || Manifest::parse(value))
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    for name in manifest.roots.keys() {
        if reserved.contains(name) {
            return Err(io::Error::other(format!("reserved root command: {name}")));
        }
    }
    let path = paths.directory.join(format!("{}.json", manifest.hash));
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let existing =
                Manifest::parse(serde_json::from_slice(&bytes).map_err(io::Error::other)?)
                    .map_err(io::Error::other)?;
            if existing.hash != manifest.hash {
                return Err(io::Error::other("cached manifest identity mismatch"));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            write_private(
                path,
                serde_json::to_vec(&manifest).map_err(io::Error::other)?,
            )
            .await?;
        }
        Err(error) => return Err(error),
    }
    let leases = leases(&manifest, residents).await;
    Ok(Installed {
        manifest: Arc::new(manifest),
        leases,
    })
}

/// Leases on the residents of `manifest`'s packages for this host.
pub async fn leases(manifest: &Manifest, residents: &ResidentHandle) -> PackageLeases {
    let mut leases = PackageLeases::new();
    for package in manifest.packages.values() {
        if let Some(artifact) = package.targets.get(host_target()) {
            let lease = residents.lease(artifact.sha256.clone()).await;
            leases.insert(package.id.clone(), lease);
        }
    }
    leases
}

/// A private directory with one alias of the runner per root command, made
/// in one blocking call, off the control thread.
/// A context's directory of aliases, which goes with the context unless the
/// runner left the context's job to the next.
struct Aliases {
    path: PathBuf,
    kept: AtomicBool,
}

impl Aliases {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            kept: AtomicBool::new(false),
        }
    }
}

impl Drop for Aliases {
    fn drop(&mut self) {
        if !self.kept.load(Ordering::SeqCst) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

async fn aliases(
    directory: PathBuf,
    executable: PathBuf,
    roots: Vec<String>,
) -> io::Result<Aliases> {
    tokio::task::spawn_blocking(move || {
        let mut builder = tempfile::Builder::new();
        builder.prefix("client-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        // Kept until the context's own drop, which a runner that takes the
        // job over makes again (`ExecutionContext::restore`).
        let aliases = Aliases::new(builder.tempdir_in(&directory)?.keep());
        for root in roots {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&executable, aliases.path.join(root))?;
            #[cfg(windows)]
            {
                let destination = aliases.path.join(format!("{root}.exe"));
                match std::fs::hard_link(&executable, &destination) {
                    Ok(()) => {}
                    // Windows hard links cannot cross volumes. A job-owned copy
                    // also works without requiring symbolic-link privileges.
                    Err(error) if error.raw_os_error() == Some(17) => {
                        std::fs::copy(&executable, destination)?;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(aliases)
    })
    .await
    .map_err(io::Error::other)?
}
