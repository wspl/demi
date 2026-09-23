//! Where artifacts are (`native-runtime.md` § Install the selected
//! executable): the backend answers only for live work on the connection
//! that the artifact belongs to, a job or an open user stream.

use crate::commands::contexts::Contexts;
use crate::connection::{ConnectionHandle, wire};
use crate::services::{ArtifactResolver, ArtifactSource, RuntimeError};
use demi_command_service::protocol::PackageArtifact;
use futures_util::future::BoxFuture;
use tokio_util::sync::CancellationToken;

/// For a job's command: any live job whose manifest carries the artifact
/// authorizes it.
pub struct JobArtifacts {
    contexts: Contexts,
}

impl JobArtifacts {
    pub fn new(contexts: Contexts) -> Self {
        Self { contexts }
    }
}

impl ArtifactResolver for JobArtifacts {
    fn resolve<'a>(
        &'a self,
        artifact: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            loop {
                let context = self.contexts.carrying(&artifact.sha256).ok_or_else(|| {
                    RuntimeError::Location("no live job authorizes this artifact".into())
                })?;
                let owner = wire::ArtifactOwner::Job(wire::JobArtifactOwner {
                    job_id: context.job_id.clone(),
                    manifest_hash: context.manifest.hash.clone(),
                });
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(RuntimeError::Cancelled),
                    // A shared download can select another still-live
                    // authorized job.
                    _ = context.cancel.cancelled() => continue,
                    location = context.connection.locate(owner, artifact.sha256.clone()) => {
                        return ArtifactSource::from_location(location?);
                    }
                }
            }
        })
    }
}

/// For a user stream's service (`runner.md` § Service streams): the open
/// stream authorizes its artifact.
pub struct StreamArtifacts {
    connection: ConnectionHandle,
    stream: String,
}

impl StreamArtifacts {
    pub fn new(connection: ConnectionHandle, stream: String) -> Self {
        Self { connection, stream }
    }
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
            tokio::select! {
                _ = cancel.cancelled() => Err(RuntimeError::Cancelled),
                location = self.connection.locate(owner, artifact.sha256.clone()) => {
                    ArtifactSource::from_location(location?)
                }
            }
        })
    }
}
