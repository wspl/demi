//! The composition root: `Backend::start` opens storage, starts the shared
//! services and the shard threads, and opens the listener;
//! `Backend::close` shuts them down in order (`backend.md` § Startup and
//! shutdown).

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use crate::auth::email_change::{AccountMail, EmailChanges};
use crate::auth::login_limiter::LoginLimiter;
use crate::auth::passwords::{HashError, PasswordHasher};
use crate::auth::sessions::WebSessions;
use crate::config::BackendConfig;
use crate::edge::Edge;
use crate::shard::ShardPool;
use crate::storage::StorageError;
use crate::storage::control::ControlService;
use crate::vault::secret::{InstanceSecret, SecretError};

const CONTROL_DATABASE: &str = "control.sqlite";

/// The services that span users, or that are needed before the user is
/// known. The edge and every shard share them.
pub(crate) struct Services {
    pub(crate) control: ControlService,
    pub(crate) hasher: PasswordHasher,
    pub(crate) sessions: WebSessions,
    pub(crate) limiter: LoginLimiter,
    pub(crate) email: EmailChanges,
}

impl Services {
    async fn start(
        control: ControlService,
        secret: &InstanceSecret,
        mail: Option<Arc<dyn AccountMail>>,
    ) -> Result<Self, HashError> {
        let hasher = PasswordHasher::new().await?;
        Ok(Self {
            sessions: WebSessions::new(control.clone()),
            limiter: LoginLimiter::new(),
            email: EmailChanges::new(control.clone(), hasher.clone(), mail, secret.email_code_key()),
            hasher,
            control,
        })
    }

    /// The services over a control database in `data`, for unit tests.
    #[cfg(test)]
    pub(crate) async fn start_for_tests(data: &std::path::Path) -> Arc<Self> {
        let control = ControlService::open(&data.join(CONTROL_DATABASE), Arc::new(crate::clock::SystemClock))
            .await
            .unwrap();
        let secret = InstanceSecret::load_or_create(data).await.unwrap();
        Arc::new(Self::start(control, &secret, None).await.unwrap())
    }
}

/// A running backend.
pub struct Backend {
    local_addr: SocketAddr,
    services: Arc<Services>,
    shards: ShardPool,
    edge: Edge,
}

/// Why the backend did not start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("the data directory {} cannot be created: {source}", path.display())]
    DataDirectory { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error("the control database cannot be opened: {0}")]
    Control(#[from] StorageError),
    #[error("password hashing cannot start: {0}")]
    Hashing(#[from] HashError),
    #[error("the shard threads cannot start: {0}")]
    Shards(io::Error),
    #[error("the backend cannot listen on {address}: {source}")]
    Listen { address: SocketAddr, source: io::Error },
}

/// A shutdown step that failed; the steps after it ran all the same.
#[derive(Debug, thiserror::Error)]
pub enum ShutdownError {
    #[error("the listener did not stop cleanly: {0}")]
    Edge(io::Error),
    #[error("the control database did not close: {0}")]
    Control(StorageError),
}

/// Every shutdown step that failed.
#[derive(Debug)]
pub struct ShutdownErrors(pub Vec<ShutdownError>);

impl fmt::Display for ShutdownErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let steps: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        write!(f, "shutdown failed: {}", steps.join("; "))
    }
}

impl std::error::Error for ShutdownErrors {}

impl Backend {
    /// Starts the backend. It serves once this returns: the data directory
    /// and the instance secret, then the control database, the shared
    /// services and the shard threads, and the listener last.
    pub async fn start(config: BackendConfig) -> Result<Self, StartError> {
        let data_dir = config.data_dir.clone();
        tokio::fs::create_dir_all(&data_dir)
            .await
            .map_err(|source| StartError::DataDirectory { path: data_dir.clone(), source })?;
        let secret = match &config.instance_secret {
            Some(secret) => secret.clone(),
            None => InstanceSecret::load_or_create(&data_dir).await?,
        };
        let control = ControlService::open(&data_dir.join(CONTROL_DATABASE), config.clock.clone()).await?;
        let started = Self::serve(config, control.clone(), &secret).await;
        if started.is_err()
            && let Err(error) = control.close().await
        {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the control database did not close after a failed start"
            );
        }
        started
    }

    async fn serve(config: BackendConfig, control: ControlService, secret: &InstanceSecret) -> Result<Self, StartError> {
        let services = Arc::new(Services::start(control, secret, config.account_mail).await?);
        let shards = ShardPool::start(config.shards, services.clone())
            .await
            .map_err(StartError::Shards)?;
        let edge = match Edge::start(config.address, services.clone(), config.web_directory).await {
            Ok(edge) => edge,
            Err(source) => {
                shards.close().await;
                return Err(StartError::Listen {
                    address: config.address,
                    source,
                });
            }
        };
        Ok(Self {
            local_addr: edge.local_addr(),
            services,
            shards,
            edge,
        })
    }

    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Shuts the backend down. The listener closes first, so no new work
    /// starts and a new request on an open connection answers 503
    /// `backend_closing`; every step runs even when an earlier one fails, and
    /// the failures are reported together.
    pub async fn close(self) -> Result<(), ShutdownErrors> {
        let mut failures = Vec::new();
        self.edge.stop_accepting();
        self.shards.close().await;
        if let Err(error) = self.edge.close().await {
            failures.push(ShutdownError::Edge(error));
        }
        if let Err(error) = self.services.control.close().await {
            failures.push(ShutdownError::Control(error));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(ShutdownErrors(failures))
        }
    }
}
