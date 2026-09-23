//! Live command authority and immutable declaration snapshots for runner-owned jobs.

use crate::commands::command_client::{CONTEXT_ENV, ENDPOINT_ENV};
use crate::services::{ServiceHandle, ServiceLease};
use demi_runner_protocol::manifest::Manifest;
use demi_command_service::protocol::CommandContext;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
use tokio_util::sync::CancellationToken;

pub struct ExecutionContext {
    pub id: String,
    pub owner: String,
    pub job_id: String,
    /// What the backend told the job's declared commands
    /// (`native-runtime.md` § Command context).
    pub command: CommandContext,
    pub manifest: Arc<Manifest>,
    pub cancel: CancellationToken,
    pub edits: OnceLock<demi_command_service::protocol::EditContext>,
    aliases: tempfile::TempDir,
}

#[derive(Clone)]
pub struct Contexts {
    state: Arc<Mutex<State>>,
    directory: PathBuf,
    executable: PathBuf,
    services: ServiceHandle,
}

struct State {
    current: Option<Leased>,
    entries: HashMap<String, Leased<Arc<ExecutionContext>>>,
}

/// A manifest or a live context with the leases that keep its services
/// resident (`native-runtime.md` § Keep a service resident).
struct Leased<T = Arc<Manifest>> {
    value: T,
    _leases: Vec<ServiceLease>,
}

/// The task owner drops this after its process and streams finish, including errors.
pub struct Lease {
    contexts: Contexts,
    id: String,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let removed = self.contexts.state.lock().unwrap().entries.remove(&self.id);
        if let Some(live) = removed {
            live.value.cancel.cancel();
        }
    }
}

impl Contexts {
    pub async fn new(
        directory: PathBuf,
        executable: PathBuf,
        services: ServiceHandle,
    ) -> io::Result<Self> {
        tokio::fs::create_dir_all(&directory).await?;
        crate::fs::chmod(&directory, 0o700).await?;
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                current: None,
                entries: HashMap::new(),
            })),
            directory,
            executable,
            services,
        })
    }

    /// Leases on the services of `manifest`'s packages for this host.
    fn leases(&self, manifest: &Manifest) -> Vec<ServiceLease> {
        manifest
            .packages
            .values()
            .filter_map(|package| package.targets.get(crate::services::target()))
            .map(|artifact| self.services.lease(artifact.sha256.clone()))
            .collect()
    }

    pub async fn install(&self, value: serde_json::Value) -> io::Result<Arc<Manifest>> {
        let manifest = Arc::new(
            tokio::task::spawn_blocking(move || Manifest::parse(value))
                .await
                .map_err(io::Error::other)?
                .map_err(io::Error::other)?,
        );
        let builtins = brush_builtins::default_builtins::<
            brush_core::extensions::DefaultShellExtensions,
        >(brush_builtins::BuiltinSet::BashMode);
        for name in manifest.roots.keys() {
            if name == "demi-runner"
                || crate::shell::utilities::NAMES.contains(&name.as_str())
                || builtins.contains_key(name)
            {
                return Err(io::Error::other(format!("reserved root command: {name}")));
            }
        }
        let path = self.directory.join(format!("{}.json", manifest.hash));
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
                    serde_json::to_vec(&*manifest).map_err(io::Error::other)?,
                )
                .await?;
            }
            Err(error) => return Err(error),
        }
        let installed = Leased {
            _leases: self.leases(&manifest),
            value: manifest.clone(),
        };
        // The new manifest's leases count before the old one's end, so a
        // service both name stays resident.
        let replaced = self.state.lock().unwrap().current.replace(installed);
        drop(replaced);
        Ok(manifest)
    }

    pub async fn create(
        &self,
        job_id: String,
        manifest_hash: &str,
        command: CommandContext,
    ) -> io::Result<(Arc<ExecutionContext>, Lease)> {
        let manifest = self
            .state
            .lock()
            .unwrap()
            .current
            .as_ref()
            .map(|installed| installed.value.clone())
            .filter(|manifest| manifest.hash == manifest_hash)
            .ok_or_else(|| io::Error::other("job manifest is not installed"))?;
        let aliases = aliases(
            self.directory.clone(),
            self.executable.clone(),
            manifest.roots.keys().cloned().collect(),
        )
        .await?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let context = Arc::new(ExecutionContext {
            id: id.clone(),
            owner: format!("job:{job_id}"),
            job_id,
            command,
            manifest,
            cancel: CancellationToken::new(),
            edits: OnceLock::new(),
            aliases,
        });
        let leases = self.leases(&context.manifest);
        let mut state = self.state.lock().unwrap();
        if state
            .entries
            .values()
            .any(|entry| entry.value.owner == context.owner)
        {
            return Err(io::Error::other("duplicate live execution owner"));
        }
        state.entries.insert(
            id.clone(),
            Leased {
                value: context.clone(),
                _leases: leases,
            },
        );
        Ok((
            context,
            Lease {
                contexts: self.clone(),
                id,
            },
        ))
    }

    pub fn get(&self, id: &str) -> io::Result<Arc<ExecutionContext>> {
        self.state
            .lock()
            .unwrap()
            .entries
            .get(id)
            .map(|entry| entry.value.clone())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "execution context is not live on this runner",
                )
            })
    }

    pub fn for_artifact(&self, digest: &str, target: &str) -> Option<Arc<ExecutionContext>> {
        self.state
            .lock()
            .unwrap()
            .entries
            .values()
            .map(|entry| &entry.value)
            .find(|context| {
                !context.cancel.is_cancelled()
                    && context.manifest.packages.values().any(|package| {
                        package
                            .targets
                            .get(target)
                            .is_some_and(|artifact| artifact.sha256 == digest)
                    })
            })
            .cloned()
    }

    pub fn cancel_owner(&self, owner: &str) {
        for context in self
            .state
            .lock()
            .unwrap()
            .entries
            .values()
            .map(|entry| &entry.value)
            .filter(|context| context.owner == owner)
        {
            context.cancel.cancel();
        }
    }

    pub fn close(&self) {
        let closed: Vec<_> = self.state.lock().unwrap().entries.drain().collect();
        for (_, live) in closed {
            live.value.cancel.cancel();
        }
    }
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

impl ExecutionContext {
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
}
