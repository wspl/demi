//! The composition root: `Backend::start` opens storage, starts the shared
//! services and the shard threads, and opens the listener;
//! `Backend::close` shuts them down in order (`backend.md` § Startup and
//! shutdown).

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use demi_web_api::settings::InstanceMode;

use crate::auth::email_change::{AccountMail, EmailChanges};
use crate::auth::login_limiter::LoginLimiter;
use crate::auth::passwords::{HashError, PasswordHasher};
use crate::auth::sessions::WebSessions;
use crate::config::BackendConfig;
use crate::edge::{AppState, Edge};
use crate::shard::ShardPool;
use crate::storage::blobs::BlobStores;
use crate::storage::control::ControlService;
use crate::storage::conversations::{self, ConversationStores};
use crate::storage::{StorageError, objects};
use crate::vault::secret::{InstanceSecret, SecretError};

const CONTROL_DATABASE: &str = "control.sqlite";
const CONVERSATION_DATABASES: &str = "conversations";

/// The services that span users, or that are needed before the user is
/// known. The edge and every shard share them.
pub(crate) struct Services {
    pub(crate) mode: InstanceMode,
    pub(crate) control: ControlService,
    #[expect(dead_code, reason = "the agent's tree store and the conversation summaries use it")]
    pub(crate) conversations: ConversationStores,
    pub(crate) blobs: BlobStores,
    pub(crate) hasher: PasswordHasher,
    pub(crate) sessions: WebSessions,
    pub(crate) limiter: LoginLimiter,
    pub(crate) email: EmailChanges,
}

/// The databases and the object store, which open before the services and
/// close after them. Cloning it is cheap.
#[derive(Clone)]
struct Storage {
    control: ControlService,
    conversations: ConversationStores,
    blobs: BlobStores,
}

impl Storage {
    async fn open(data_dir: &Path, clock: Arc<dyn demi_core::Clock>) -> Result<Self, StorageError> {
        let control = ControlService::open(&data_dir.join(CONTROL_DATABASE), clock).await?;
        let rest = async {
            let conversations =
                ConversationStores::open(data_dir.join(CONVERSATION_DATABASES), conversations::MAX_WRITERS).await?;
            let blobs = BlobStores::new(objects::open(data_dir).await?);
            Ok::<_, StorageError>((conversations, blobs))
        }
        .await;
        match rest {
            Ok((conversations, blobs)) => Ok(Self {
                control,
                conversations,
                blobs,
            }),
            Err(error) => {
                // Opening the rest left no connection open; the control
                // database has one.
                if let Err(failure) = control.close().await {
                    tracing::error!(
                        error = &failure as &dyn std::error::Error,
                        "the control database did not close after a failed start"
                    );
                }
                Err(error)
            }
        }
    }

    /// Closes the conversation databases, then the control database, and
    /// answers every step that failed.
    async fn close(&self) -> Vec<ShutdownError> {
        let mut failures: Vec<ShutdownError> = self
            .conversations
            .close()
            .await
            .into_iter()
            .map(ShutdownError::Conversation)
            .collect();
        if let Err(error) = self.control.close().await {
            failures.push(ShutdownError::Control(error));
        }
        failures
    }
}

impl Services {
    async fn start(
        mode: InstanceMode,
        storage: Storage,
        secret: &InstanceSecret,
        mail: Option<Arc<dyn AccountMail>>,
    ) -> Result<Self, HashError> {
        let Storage {
            control,
            conversations,
            blobs,
        } = storage;
        let hasher = PasswordHasher::new().await?;
        Ok(Self {
            mode,
            sessions: WebSessions::new(control.clone()),
            limiter: LoginLimiter::new(),
            email: EmailChanges::new(control.clone(), hasher.clone(), mail, secret.email_code_key()),
            hasher,
            control,
            conversations,
            blobs,
        })
    }

    /// The services over storage in `data`, for unit tests.
    #[cfg(test)]
    pub(crate) async fn start_for_tests(data: &std::path::Path) -> Arc<Self> {
        let storage = Storage::open(data, Arc::new(demi_core::SystemClock)).await.unwrap();
        let secret = InstanceSecret::load_or_create(data).await.unwrap();
        Arc::new(Self::start(InstanceMode::Shared, storage, &secret, None).await.unwrap())
    }
}

/// A running backend.
pub struct Backend {
    local_addr: SocketAddr,
    storage: Storage,
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
    #[error("storage cannot be opened: {0}")]
    Storage(#[from] StorageError),
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
    #[error("a conversation database did not close: {0}")]
    Conversation(StorageError),
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
    /// and the instance secret, then the databases and the object store, the
    /// shared services and the shard threads, and the listener last.
    pub async fn start(config: BackendConfig) -> Result<Self, StartError> {
        let data_dir = config.data_dir.clone();
        tokio::fs::create_dir_all(&data_dir)
            .await
            .map_err(|source| StartError::DataDirectory { path: data_dir.clone(), source })?;
        let secret = match &config.instance_secret {
            Some(secret) => secret.clone(),
            None => InstanceSecret::load_or_create(&data_dir).await?,
        };
        let storage = Storage::open(&data_dir, config.clock.clone()).await?;
        let started = Self::serve(config, storage.clone(), &secret).await;
        if started.is_err() {
            for failure in storage.close().await {
                tracing::error!(
                    error = &failure as &dyn std::error::Error,
                    "storage did not close after a failed start"
                );
            }
        }
        started
    }

    async fn serve(config: BackendConfig, storage: Storage, secret: &InstanceSecret) -> Result<Self, StartError> {
        let services = Arc::new(Services::start(config.mode, storage.clone(), secret, config.account_mail).await?);
        let shards = ShardPool::start(config.shards, services.clone())
            .await
            .map_err(StartError::Shards)?;
        let state = AppState {
            services: services.clone(),
            shards: shards.shards(),
        };
        let edge = match Edge::start(config.address, state, config.web_directory).await {
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
            storage,
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
        failures.extend(self.storage.close().await);
        if failures.is_empty() {
            Ok(())
        } else {
            Err(ShutdownErrors(failures))
        }
    }
}
