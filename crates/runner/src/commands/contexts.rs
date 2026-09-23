//! Execution contexts (`native-runtime.md` § Command context): the live
//! authority a job's declared commands run under, and the manifest the
//! connection installed. The connection owns its contexts; everything else
//! looks them up in the snapshot it publishes.

use crate::commands::command_client::{CONTEXT_ENV, ENDPOINT_ENV};
use crate::connection::ConnectionHandle;
use crate::services::{ServiceHandle, ServiceLease};
use demi_command_service::protocol::{CommandContext, EditContext};
use demi_runner_protocol::manifest::Manifest;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub struct ExecutionContext {
    pub id: String,
    pub job_id: String,
    /// What the backend told the job's declared commands
    /// (`native-runtime.md` § Command context).
    pub command: CommandContext,
    pub manifest: Arc<Manifest>,
    pub cancel: CancellationToken,
    /// Where the job records the files its commands change.
    pub edits: EditContext,
    /// The connection the job runs under, for its callbacks and artifact
    /// locations.
    pub connection: ConnectionHandle,
    aliases: tempfile::TempDir,
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
            aliases,
        })
    }

    pub fn environment(
        &self,
        endpoint: &str,
        home: &str,
        path: Option<&str>,
    ) -> io::Result<BTreeMap<String, String>> {
        let mut paths = vec![self.aliases.path().to_owned()];
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

    /// Whether the context's manifest carries `digest` for this host.
    pub fn carries(&self, digest: &str) -> bool {
        self.manifest.packages.values().any(|package| {
            package
                .targets
                .get(crate::services::target())
                .is_some_and(|artifact| artifact.sha256 == digest)
        })
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
        crate::fs::chmod(&directory, 0o700).await?;
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
/// change as a new snapshot; a context's service leases end when it leaves.
pub struct ContextTable {
    index: watch::Sender<Arc<ContextIndex>>,
    leases: HashMap<String, Vec<ServiceLease>>,
}

impl ContextTable {
    pub fn new(index: watch::Sender<Arc<ContextIndex>>) -> Self {
        index.send_replace(Arc::default());
        Self {
            index,
            leases: HashMap::new(),
        }
    }

    /// Makes `context` live; a job has at most one.
    pub fn insert(
        &mut self,
        context: Arc<ExecutionContext>,
        leases: Vec<ServiceLease>,
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

    /// Ends the context of `job_id`, once its process and streams finished.
    pub fn remove(&mut self, job_id: &str) {
        let mut entries = ContextIndex::clone(&self.index.borrow());
        let Some(id) = entries
            .values()
            .find(|context| context.job_id == job_id)
            .map(|context| context.id.clone())
        else {
            return;
        };
        if let Some(context) = entries.remove(&id) {
            context.cancel.cancel();
        }
        self.leases.remove(&id);
        self.index.send_replace(Arc::new(entries));
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

/// The connection's manifest as the jobs that name one see it.
#[derive(Clone)]
pub enum Installation {
    /// None installed yet.
    Absent,
    /// The latest manifest is being checked and kept.
    Installing,
    Ready(Arc<Manifest>),
}

/// A manifest the connection installed, with the leases that keep its
/// services resident (`native-runtime.md` § Keep a service resident).
pub struct Installed {
    pub manifest: Arc<Manifest>,
    pub leases: Vec<ServiceLease>,
}

/// Parses, checks and keeps `value` as the connection's manifest.
pub async fn install(
    value: serde_json::Value,
    paths: &ContextPaths,
    services: &ServiceHandle,
) -> io::Result<Installed> {
    let manifest = tokio::task::spawn_blocking(move || Manifest::parse(value))
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    let builtins = brush_builtins::default_builtins::<
        brush_core::extensions::DefaultShellExtensions,
    >(brush_builtins::BuiltinSet::BashMode);
    for name in manifest.roots.keys() {
        if name == "demi-runner"
            || crate::shell::utilities::is_utility(name)
            || builtins.contains_key(name)
        {
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
            crate::state::write_private(
                path,
                serde_json::to_vec(&manifest).map_err(io::Error::other)?,
            )
            .await?;
        }
        Err(error) => return Err(error),
    }
    let leases = leases(&manifest, services).await;
    Ok(Installed {
        manifest: Arc::new(manifest),
        leases,
    })
}

/// Leases on the services of `manifest`'s packages for this host.
pub async fn leases(manifest: &Manifest, services: &ServiceHandle) -> Vec<ServiceLease> {
    let mut leases = Vec::new();
    for package in manifest.packages.values() {
        if let Some(artifact) = package.targets.get(crate::services::target()) {
            leases.push(services.lease(artifact.sha256.clone()).await);
        }
    }
    leases
}

/// A private directory with one alias of the runner per root command, made
/// in one blocking call, off the control thread.
async fn aliases(
    directory: PathBuf,
    executable: PathBuf,
    roots: Vec<String>,
) -> io::Result<tempfile::TempDir> {
    tokio::task::spawn_blocking(move || {
        let mut builder = tempfile::Builder::new();
        builder.prefix("client-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let aliases = builder.tempdir_in(&directory)?;
        for root in roots {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&executable, aliases.path().join(root))?;
            #[cfg(windows)]
            {
                let destination = aliases.path().join(format!("{root}.exe"));
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
