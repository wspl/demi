//! Exact artifact location requests over the authenticated backend connection.

use crate::contexts::Contexts;
use demi_command_protocol::PackageArtifact;
use demi_command_runtime::{ArtifactResolver, ArtifactSource, RuntimeError};
use demi_runner_protocol::{self as wire, ArtifactLocationLocation};
use futures_util::future::BoxFuture;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

type Location = Result<ArtifactLocationLocation, String>;
struct Connection {
    output: mpsc::Sender<wire::Outbound>,
    stop: CancellationToken,
}
struct State {
    connection: Option<Connection>,
    pending: HashMap<String, oneshot::Sender<Location>>,
}

pub struct Artifacts {
    contexts: Contexts,
    target: String,
    state: Arc<Mutex<State>>,
}

struct Pending {
    id: String,
    state: Arc<Mutex<State>>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.state.lock().unwrap().pending.remove(&self.id);
    }
}

impl Artifacts {
    pub fn new(contexts: Contexts, target: String) -> Arc<Self> {
        Arc::new(Self {
            contexts,
            target,
            state: Arc::new(Mutex::new(State {
                connection: None,
                pending: HashMap::new(),
            })),
        })
    }
    pub fn attach(&self, output: mpsc::Sender<wire::Outbound>) {
        self.detach();
        self.state.lock().unwrap().connection = Some(Connection {
            output,
            stop: CancellationToken::new(),
        });
    }
    pub fn detach(&self) {
        let mut state = self.state.lock().unwrap();
        if let Some(connection) = state.connection.take() {
            connection.stop.cancel();
        }
        state.pending.clear();
    }
    pub fn reply(
        &self,
        id: &str,
        location: Option<ArtifactLocationLocation>,
        error: Option<String>,
    ) {
        let Some(sender) = self.state.lock().unwrap().pending.remove(id) else {
            return;
        };
        let result = match (location, error) {
            (Some(location), None) => Ok(location),
            (None, Some(error)) => Err(error),
            _ => Err("invalid artifact location response".into()),
        };
        // Dropped receivers mean the requesting context was cancelled.
        let _cancelled_request = sender.send(result);
    }

    async fn request(
        &self,
        artifact: &PackageArtifact,
        cancel: &CancellationToken,
    ) -> Result<ArtifactSource, RuntimeError> {
        loop {
            let context = self
                .contexts
                .for_artifact(&artifact.sha256, &self.target)
                .ok_or_else(|| {
                    RuntimeError::Artifact("no live job authorizes this artifact".into())
                })?;
            let id = uuid::Uuid::new_v4().simple().to_string();
            let (sender, receiver) = oneshot::channel();
            let (output, stop) = {
                let mut state = self.state.lock().unwrap();
                if state.pending.len() >= 32 {
                    return Err(RuntimeError::Artifact(
                        "artifact request limit reached".into(),
                    ));
                }
                let connection = state.connection.as_ref().ok_or(RuntimeError::Cancelled)?;
                let output = connection.output.clone();
                let stop = connection.stop.clone();
                state.pending.insert(id.clone(), sender);
                (output, stop)
            };
            let _pending = Pending {
                id: id.clone(),
                state: self.state.clone(),
            };
            let message = wire::artifact_resolve(
                id,
                context.job_id.clone(),
                context.manifest.hash.clone(),
                artifact.sha256.clone(),
                self.target.clone(),
            )
            .map_err(|error| RuntimeError::Artifact(error.to_string()))?;
            let location = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(RuntimeError::Cancelled),
                _ = stop.cancelled() => return Err(RuntimeError::Cancelled),
                // A shared download can select another still-live authorized job.
                _ = context.cancel.cancelled() => continue,
                result = tokio::time::timeout(Duration::from_secs(15), async {
                    output.send(message).await.map_err(|_| RuntimeError::Cancelled)?;
                    receiver.await.map_err(|_| RuntimeError::Cancelled)?.map_err(RuntimeError::Artifact)
                }) => result.map_err(|_| RuntimeError::Deadline("artifact location"))??,
            };
            return match location {
                ArtifactLocationLocation::Variant0(location) => {
                    let url = reqwest::Url::parse(&location.url)
                        .map_err(|error| RuntimeError::Artifact(error.to_string()))?;
                    if url.scheme() != "https"
                        || !url.username().is_empty()
                        || url.password().is_some()
                    {
                        return Err(RuntimeError::Artifact(
                            "artifact downloads require an HTTPS URL without credentials".into(),
                        ));
                    }
                    let expires_at = location
                        .expires_at
                        .map(|millis| {
                            u64::try_from(millis)
                                .ok()
                                .and_then(|millis| {
                                    UNIX_EPOCH.checked_add(Duration::from_millis(millis))
                                })
                                .ok_or_else(|| {
                                    RuntimeError::Artifact("invalid artifact URL expiry".into())
                                })
                        })
                        .transpose()?;
                    Ok(ArtifactSource::Https {
                        url: url.into(),
                        expires_at,
                    })
                }
                ArtifactLocationLocation::Variant1(location) => {
                    let path = std::path::PathBuf::from(location.path);
                    if !path.is_absolute() {
                        return Err(RuntimeError::Artifact(
                            "local artifact path must be absolute".into(),
                        ));
                    }
                    Ok(ArtifactSource::Local(path))
                }
            };
        }
    }
}

impl ArtifactResolver for Artifacts {
    fn resolve<'a>(
        &'a self,
        artifact: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(self.request(artifact, cancel))
    }
}
