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

use demi_provider::models_dev::ModelsDevClient;

use crate::auth::email_change::{AccountMail, EmailChanges};
use crate::auth::login_limiter::LoginLimiter;
use crate::auth::passwords::{HashError, PasswordHasher};
use crate::auth::sessions::WebSessions;
use crate::config::{BackendConfig, ConversationTuning, RunnerTuning};
use crate::edge::{AppState, Edge, Installation};
use crate::llm::assembly::ProviderAssembly;
use crate::llm::catalog_cache::ModelCatalogCache;
use crate::llm::families::FamilyRegistry;
use crate::llm::vendors::VendorCatalog;
use crate::runner::claims::PendingClaims;
use crate::shard::ShardPool;
use crate::storage::blobs::BlobStores;
use crate::storage::control::ControlService;
use crate::storage::conversations::{self, ConversationStores};
use crate::storage::{StorageError, objects};
use crate::vault::entries::Vault;
use crate::vault::logins::{LoginFlows, LoginTiming};
use crate::vault::operations::ProviderOperations;
use crate::vault::quotas::AccountQuotas;
use crate::vault::secret::{InstanceSecret, SecretError};

const CONTROL_DATABASE: &str = "control.sqlite";
const CONVERSATION_DATABASES: &str = "conversations";

/// The services that span users, or that are needed before the user is
/// known. The edge and every shard share them.
pub(crate) struct Services {
    pub(crate) mode: InstanceMode,
    /// The wall clock the backend reads times from.
    pub(crate) clock: Arc<dyn demi_core::Clock>,
    pub(crate) control: ControlService,
    pub(crate) conversations: ConversationStores,
    pub(crate) blobs: BlobStores,
    pub(crate) hasher: PasswordHasher,
    pub(crate) sessions: WebSessions,
    pub(crate) limiter: LoginLimiter,
    pub(crate) email: EmailChanges,
    pub(crate) vault: Vault,
    pub(crate) assembly: Arc<ProviderAssembly>,
    pub(crate) operations: Arc<ProviderOperations>,
    pub(crate) logins: Arc<LoginFlows>,
    /// Runners waiting to be paired, which have no user yet.
    pub(crate) claims: PendingClaims,
    pub(crate) runners: RunnerTuning,
    pub(crate) conversation_tuning: ConversationTuning,
}

/// What the provider services start with.
pub(crate) struct ProviderSetup {
    pub(crate) families: FamilyRegistry,
    pub(crate) models_dev_url: url::Url,
    pub(crate) logins: LoginTiming,
    pub(crate) clock: Arc<dyn demi_core::Clock>,
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
    /// Starts the services on the edge's runtime, which the ones that spawn
    /// work of their own run it on.
    async fn start(
        mode: InstanceMode,
        storage: Storage,
        secret: &InstanceSecret,
        mail: Option<Arc<dyn AccountMail>>,
        providers: ProviderSetup,
        runners: RunnerTuning,
        conversation_tuning: ConversationTuning,
    ) -> Result<Self, StartError> {
        let Storage {
            control,
            conversations,
            blobs,
        } = storage;
        let hasher = PasswordHasher::new().await?;
        let clock = providers.clock.clone();
        let edge = tokio::runtime::Handle::current();
        // The shared services' own client; each shard thread builds its own.
        let http = reqwest::Client::builder().build().map_err(StartError::Http)?;
        let vault = Vault::new(control.clone(), secret.vault_key(), mode);
        let models_dev = ModelsDevClient::new(http.clone(), providers.models_dev_url, providers.clock.clone());
        let assembly = Arc::new(ProviderAssembly::new(
            vault.clone(),
            providers.families,
            AccountQuotas::new(control.clone(), edge.clone()),
            ModelCatalogCache::new(control.clone(), providers.clock.clone(), edge),
            VendorCatalog::new(models_dev),
            http,
            providers.clock,
        ));
        let operations = Arc::new(ProviderOperations::default());
        let logins = LoginFlows::new(assembly.clone(), operations.clone(), providers.logins);
        Ok(Self {
            mode,
            clock,
            sessions: WebSessions::new(control.clone()),
            limiter: LoginLimiter::new(),
            email: EmailChanges::new(control.clone(), hasher.clone(), mail, secret.email_code_key()),
            hasher,
            control,
            conversations,
            blobs,
            vault,
            assembly,
            operations,
            logins,
            claims: PendingClaims::new(runners.claims_per_minute),
            runners,
            conversation_tuning,
        })
    }

    /// Cancels the logins, and drains the catalog refreshes and quota
    /// writes, before storage closes.
    async fn close_providers(&self) {
        self.logins.close().await;
        self.assembly.close().await;
    }

    /// The services over storage in `data`, for unit tests.
    #[cfg(test)]
    pub(crate) async fn start_for_tests(data: &std::path::Path) -> Arc<Self> {
        let clock: Arc<dyn demi_core::Clock> = Arc::new(demi_core::SystemClock);
        let storage = Storage::open(data, clock.clone()).await.unwrap();
        let secret = InstanceSecret::load_or_create(data).await.unwrap();
        let providers = ProviderSetup {
            families: FamilyRegistry::builtin(),
            models_dev_url: ModelsDevClient::DEFAULT_URL.parse().unwrap(),
            logins: LoginTiming::default(),
            clock,
        };
        let services = Self::start(
            InstanceMode::Shared,
            storage,
            &secret,
            None,
            providers,
            RunnerTuning::default(),
            ConversationTuning::default(),
        );
        Arc::new(services.await.unwrap())
    }
}

/// A running backend.
pub struct Backend {
    local_addr: SocketAddr,
    storage: Storage,
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
    #[error("storage cannot be opened: {0}")]
    Storage(#[from] StorageError),
    #[error("password hashing cannot start: {0}")]
    Hashing(#[from] HashError),
    #[error("the HTTP client cannot start: {0}")]
    Http(reqwest::Error),
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
        let providers = ProviderSetup {
            families: config.families,
            models_dev_url: config.models_dev_url,
            logins: config.logins,
            clock: config.clock,
        };
        let services = Services::start(
            config.mode,
            storage.clone(),
            secret,
            config.account_mail,
            providers,
            config.runners,
            config.conversations,
        );
        let services = Arc::new(services.await?);
        let shards = match ShardPool::start(config.shards, services.clone()).await {
            Ok(shards) => shards,
            Err(error) => {
                services.close_providers().await;
                return Err(StartError::Shards(error));
            }
        };
        let state = AppState {
            services: services.clone(),
            shards: shards.shards(),
            installation: Arc::new(Installation {
                releases: config.runner_releases,
                public_url: config.public_url,
            }),
        };
        let edge = match Edge::start(config.address, state, config.web_directory).await {
            Ok(edge) => edge,
            Err(source) => {
                shards.close().await;
                services.close_providers().await;
                return Err(StartError::Listen {
                    address: config.address,
                    source,
                });
            }
        };
        Ok(Self {
            local_addr: edge.local_addr(),
            storage,
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
        self.services.logins.close().await;
        self.shards.close().await;
        self.services.claims.close();
        if let Err(error) = self.edge.close().await {
            failures.push(ShutdownError::Edge(error));
        }
        self.services.assembly.close().await;
        failures.extend(self.storage.close().await);
        if failures.is_empty() {
            Ok(())
        } else {
            Err(ShutdownErrors(failures))
        }
    }
}
