//! Live command authority and immutable declaration snapshots for runner-owned jobs.

use crate::commands::command_client::{CONTEXT_ENV, ENDPOINT_ENV};
use crate::commands::manifest::Manifest;
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
    pub agent_session_id: String,
    pub shell_id: String,
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
    changed: Arc<tokio::sync::Notify>,
}

struct State {
    current: Option<Arc<Manifest>>,
    entries: HashMap<String, Arc<ExecutionContext>>,
}

/// The task owner drops this after its process and streams finish, including errors.
pub struct Lease {
    contexts: Contexts,
    id: String,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(context) = self.contexts.state.lock().unwrap().entries.remove(&self.id) {
            context.cancel.cancel();
            self.contexts.changed.notify_one();
        }
    }
}

impl Contexts {
    pub async fn new(directory: PathBuf, executable: PathBuf) -> io::Result<Self> {
        tokio::fs::create_dir_all(&directory).await?;
        crate::fs::chmod(&directory, 0o700).await?;
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                current: None,
                entries: HashMap::new(),
            })),
            directory,
            executable,
            changed: Arc::new(tokio::sync::Notify::new()),
        })
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
        self.state.lock().unwrap().current = Some(manifest.clone());
        self.changed.notify_one();
        Ok(manifest)
    }

    pub async fn create(
        &self,
        job_id: String,
        manifest_hash: &str,
        env: &BTreeMap<String, String>,
    ) -> io::Result<(Arc<ExecutionContext>, Lease)> {
        let manifest = self
            .state
            .lock()
            .unwrap()
            .current
            .clone()
            .filter(|manifest| manifest.hash == manifest_hash)
            .ok_or_else(|| io::Error::other("job manifest is not installed"))?;
        let aliases = tempfile::Builder::new()
            .prefix("client-")
            .tempdir_in(&self.directory)?;
        crate::fs::chmod(aliases.path(), 0o700).await?;
        for root in manifest.roots.keys() {
            #[cfg(unix)]
            tokio::fs::symlink(&self.executable, aliases.path().join(root)).await?;
            #[cfg(windows)]
            {
                let destination = aliases.path().join(format!("{root}.exe"));
                match tokio::fs::hard_link(&self.executable, &destination).await {
                    Ok(()) => {}
                    // Windows hard links cannot cross volumes. A job-owned copy
                    // also works without requiring symbolic-link privileges.
                    Err(error) if error.raw_os_error() == Some(17) => {
                        tokio::fs::copy(&self.executable, destination).await?;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let context = Arc::new(ExecutionContext {
            id: id.clone(),
            owner: format!("job:{job_id}"),
            job_id,
            agent_session_id: env.get("DEMI_SESSION_ID").cloned().unwrap_or_default(),
            shell_id: env.get("DEMI_SHELL_ID").cloned().unwrap_or_default(),
            manifest,
            cancel: CancellationToken::new(),
            edits: OnceLock::new(),
            aliases,
        });
        let mut state = self.state.lock().unwrap();
        if state
            .entries
            .values()
            .any(|entry| entry.owner == context.owner)
        {
            return Err(io::Error::other("duplicate live execution owner"));
        }
        state.entries.insert(id.clone(), context.clone());
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
            .cloned()
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

    pub fn retained_artifacts(&self, target: &str) -> std::collections::HashSet<String> {
        let state = self.state.lock().unwrap();
        state
            .current
            .iter()
            .chain(state.entries.values().map(|context| &context.manifest))
            .flat_map(|manifest| manifest.packages.values())
            .filter_map(|package| {
                package
                    .targets
                    .get(target)
                    .map(|artifact| artifact.sha256.clone())
            })
            .collect()
    }

    pub fn cancel_owner(&self, owner: &str) {
        for context in self
            .state
            .lock()
            .unwrap()
            .entries
            .values()
            .filter(|context| context.owner == owner)
        {
            context.cancel.cancel();
        }
    }

    pub async fn changed(&self) {
        self.changed.notified().await;
    }

    pub fn close(&self) {
        for (_, context) in self.state.lock().unwrap().entries.drain() {
            context.cancel.cancel();
        }
        self.changed.notify_one();
    }
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
