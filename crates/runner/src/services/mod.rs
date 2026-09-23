//! Resident native services (`native-runtime.md` § Invoke and retire a
//! service): the verified executable cache, the service processes, and the
//! registry that keeps each one resident while something holds a lease on it.

pub mod cache;
pub mod process;
pub mod registry;

use std::{fmt, path::PathBuf, time::SystemTime};

use demi_command_service::protocol::{ArtifactLocation, PackageArtifact};
use futures_util::future::BoxFuture;
use tokio_util::sync::CancellationToken;

pub use demi_command_service::protocol::host_target as target;
pub use registry::{Resident, ServiceHandle, ServiceLease, ServiceRegistry};

/// Why a resident service could not be had, or why it ended.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The executable could not be downloaded, copied or verified.
    #[error("native artifact: {0}")]
    Artifact(#[from] demi_artifact::Error),
    /// The backend did not say where the artifact is, or said it wrongly.
    #[error("native artifact location: {0}")]
    Location(String),
    #[error("native runtime was cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Service(#[from] demi_command_service::ServiceError),
    #[error("native service does not match its package descriptor")]
    CatalogMismatch,
    #[error("native service did not answer within its {0} deadline")]
    Deadline(&'static str),
    /// The registry shut the service down: nothing held it any longer, or
    /// the backend connection ended.
    #[error("native service was shut down")]
    Stopped,
    /// The service ended on its own.
    #[error(transparent)]
    Exited(#[from] ServiceExit),
}

/// How a resident service ended on its own, which every call that fails with
/// it reports (`native-runtime.md` § Invocation protocol).
#[derive(Debug, Clone, thiserror::Error)]
pub struct ServiceExit {
    /// The package, such as `demi.builtin`.
    pub service: String,
    pub reason: ExitReason,
    /// The end of the service's standard error.
    pub stderr: String,
}

impl fmt::Display for ServiceExit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "native service {} {}", self.service, self.reason)?;
        let stderr = self.stderr.trim_end();
        if !stderr.is_empty() {
            write!(formatter, "; its standard error ended with:\n{stderr}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ExitReason {
    #[error("exited with {0}")]
    Exited(std::process::ExitStatus),
    /// The runner stopped it after it broke the protocol.
    #[error("broke the protocol ({0}) and was stopped")]
    Protocol(String),
    /// The runner stopped it after it missed a deadline while starting.
    #[error("did not answer within its {0} deadline and was stopped")]
    Deadline(&'static str),
}

/// Where the runner fetches an artifact from.
pub enum ArtifactSource {
    Local(PathBuf),
    Https {
        url: String,
        expires_at: Option<SystemTime>,
    },
}

impl ArtifactSource {
    /// A location the backend returned, checked the way downloads require:
    /// an HTTPS URL without credentials, or an absolute local path.
    pub fn from_location(location: ArtifactLocation) -> Result<Self, RuntimeError> {
        match location {
            ArtifactLocation::Url(location) => {
                let url = reqwest::Url::parse(&location.url)
                    .map_err(|error| RuntimeError::Location(error.to_string()))?;
                if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
                {
                    return Err(RuntimeError::Location(
                        "artifact downloads require an HTTPS URL without credentials".into(),
                    ));
                }
                let expires_at = location
                    .expires_at
                    .map(|millis| {
                        u64::try_from(millis)
                            .ok()
                            .and_then(|millis| {
                                std::time::UNIX_EPOCH
                                    .checked_add(std::time::Duration::from_millis(millis))
                            })
                            .ok_or_else(|| {
                                RuntimeError::Location("invalid artifact URL expiry".into())
                            })
                    })
                    .transpose()?;
                Ok(ArtifactSource::Https {
                    url: url.into(),
                    expires_at,
                })
            }
            ArtifactLocation::Path(location) => {
                let path = PathBuf::from(location.path);
                if !path.is_absolute() {
                    return Err(RuntimeError::Location(
                        "local artifact path must be absolute".into(),
                    ));
                }
                Ok(ArtifactSource::Local(path))
            }
        }
    }
}

/// Resolves only an artifact authorized by the calling registration's catalog.
/// URLs are resolved again for each download attempt and never become cache keys.
pub trait ArtifactResolver: Send + Sync + 'static {
    fn resolve<'a>(
        &'a self,
        artifact: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>>;
}
