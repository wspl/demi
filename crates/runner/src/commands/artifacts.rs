//! Exact artifact location requests over the authenticated backend connection.

use crate::services::{ArtifactResolver, ArtifactSource, RuntimeError};
use crate::commands::contexts::Contexts;
use crate::connection::wire;
use demi_command_service::protocol::ArtifactLocation;
use demi_command_service::protocol::PackageArtifact;
use futures_util::future::BoxFuture;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

type Location = Result<ArtifactLocation, String>;
struct Connection {
    output: mpsc::Sender<wire::Frame>,
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
    pub fn attach(&self, output: mpsc::Sender<wire::Frame>) {
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
    pub fn reply(&self, id: &str, location: Option<ArtifactLocation>, error: Option<String>) {
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

    /// The resolver for a user stream's service (`runner.md` § Service
    /// streams): the backend authorizes its artifact by the open stream.
    pub fn for_stream(self: &Arc<Self>, stream: String) -> Arc<dyn ArtifactResolver> {
        Arc::new(StreamArtifacts {
            artifacts: self.clone(),
            stream,
        })
    }

    /// Asks the backend where the artifact is on behalf of `owner`, the live
    /// work that authorizes it; `None` once `ended`, when the owner no longer
    /// does.
    async fn locate(
        &self,
        owner: wire::ArtifactOwner,
        artifact: &PackageArtifact,
        cancel: &CancellationToken,
        ended: impl std::future::Future<Output = ()>,
    ) -> Result<Option<ArtifactSource>, RuntimeError> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let (sender, receiver) = oneshot::channel();
        let (output, stop) = {
            let mut state = self.state.lock().unwrap();
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
        let message =
            wire::encode(&wire::Outbound::ArtifactResolve {
                id,
                owner,
                sha256: artifact.sha256.clone(),
                target: self.target.clone(),
            })
                .map_err(|error| RuntimeError::Location(error.to_string()))?;
        let location = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(RuntimeError::Cancelled),
            _ = stop.cancelled() => return Err(RuntimeError::Cancelled),
            _ = ended => return Ok(None),
            result = tokio::time::timeout(Duration::from_secs(15), async {
                output.send(message).await.map_err(|_| RuntimeError::Cancelled)?;
                receiver.await.map_err(|_| RuntimeError::Cancelled)?.map_err(RuntimeError::Location)
            }) => result.map_err(|_| RuntimeError::Deadline("artifact location"))??,
        };
        ArtifactSource::from_location(location).map(Some)
    }
}

impl ArtifactResolver for Artifacts {
    /// A job's command: any live job whose catalog holds the artifact
    /// authorizes it.
    fn resolve<'a>(
        &'a self,
        artifact: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            loop {
                let context = self
                    .contexts
                    .for_artifact(&artifact.sha256, &self.target)
                    .ok_or_else(|| {
                        RuntimeError::Location("no live job authorizes this artifact".into())
                    })?;
                let owner =
                    wire::ArtifactOwner::Job(wire::JobArtifactOwner {
                        job_id: context.job_id.clone(),
                        manifest_hash: context.manifest.hash.clone(),
                    });
                // A shared download can select another still-live authorized job.
                if let Some(source) = self
                    .locate(owner, artifact, cancel, context.cancel.cancelled())
                    .await?
                {
                    return Ok(source);
                }
            }
        })
    }
}

struct StreamArtifacts {
    artifacts: Arc<Artifacts>,
    stream: String,
}

impl ArtifactResolver for StreamArtifacts {
    fn resolve<'a>(
        &'a self,
        artifact: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            let owner = wire::ArtifactOwner::Stream(wire::StreamArtifactOwner {
                stream_id: self.stream.clone(),
            });
            self.artifacts
                .locate(owner, artifact, cancel, std::future::pending())
                .await?
                .ok_or(RuntimeError::Cancelled)
        })
    }
}
