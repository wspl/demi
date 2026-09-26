//! The composition root: `Backend::start` opens storage, starts the shared
//! services and the shard threads, and opens the listener;
//! `Backend::close` shuts them down in order (`backend.md` § Startup and
//! shutdown).

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio_util::task::AbortOnDropHandle;

use demi_command_tree::NativeOperation;
use demi_web_api::settings::InstanceMode;

use demi_provider::models_dev::ModelsDevClient;

use crate::auth::email_change::{AccountMail, EmailChanges};
use crate::auth::login_limiter::LoginLimiter;
use crate::auth::passwords::{HashError, PasswordHasher};
use crate::auth::sessions::WebSessions;
use crate::config::{BackendConfig, ConversationTuning, ExposeTuning, LifecycleTuning, RunnerTuning};
use crate::conversation::stream::UserStreams;
use crate::edge::{AppState, Edge, Site};
use crate::expose::ExposeDomain;
use crate::llm::assembly::ProviderAssembly;
use crate::llm::catalog_cache::ModelCatalogCache;
use crate::llm::claude_cli::CliInstalls;
use crate::llm::claude_releases::ClaudeReleases;
use crate::llm::families::FamilyRegistry;
use crate::llm::vendors::VendorCatalog;
use crate::managed::{CloudServices, MachinesClient, recover_resets};
use crate::runner::claims::PendingClaims;
use crate::runner::native::NativeCatalog;
use crate::shard::ShardPool;
use crate::storage::blobs::BlobStores;
use crate::storage::changes::ChangeStore;
use crate::storage::objects::{S3Config, S3ConfigError};
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
    /// The retained edits of every conversation's commands.
    pub(crate) changes: ChangeStore,
    pub(crate) hasher: PasswordHasher,
    pub(crate) sessions: WebSessions,
    pub(crate) limiter: LoginLimiter,
    pub(crate) email: EmailChanges,
    pub(crate) vault: Vault,
    pub(crate) assembly: Arc<ProviderAssembly>,
    /// The vendor's Claude Code releases, which the CLI on each Cloud follows.
    pub(crate) claude_releases: ClaudeReleases,
    /// The outcome of the CLI installs no conversation asked for.
    pub(crate) cli_installs: CliInstalls,
    pub(crate) operations: Arc<ProviderOperations>,
    pub(crate) logins: Arc<LoginFlows>,
    /// Runners waiting to be paired, which have no user yet.
    pub(crate) claims: PendingClaims,
    pub(crate) runners: RunnerTuning,
    pub(crate) conversation_tuning: ConversationTuning,
    /// The native packages each shard's catalog is built from.
    pub(crate) native: NativeCatalog,
    /// The user streams a page may open.
    pub(crate) user_streams: UserStreams,
    /// The machine manager's client, the Cloud capacity across users and
    /// the Cloud's settings.
    pub(crate) cloud: CloudServices,
    /// When a conversation's Host resources are reclaimed.
    pub(crate) lifecycle: LifecycleTuning,
    /// The domain of expose hostnames; without it, exposes are unavailable.
    pub(crate) expose_domain: Option<ExposeDomain>,
    /// How the public relay treats its connections.
    pub(crate) expose_tuning: ExposeTuning,
}

/// What the provider services start with.
pub(crate) struct ProviderSetup {
    pub(crate) families: FamilyRegistry,
    pub(crate) models_dev_url: url::Url,
    /// The Claude Code distribution.
    pub(crate) claude_releases: url::Url,
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
    changes: ChangeStore,
}

impl Storage {
    /// The databases in `data_dir`, and the object store: the S3 bucket `s3`
    /// names, or the data directory.
    async fn open(data_dir: &Path, clock: Arc<dyn demi_core::Clock>, s3: Option<&S3Config>) -> Result<Self, StorageError> {
        let control = ControlService::open(&data_dir.join(CONTROL_DATABASE), clock).await?;
        let rest = async {
            let conversations =
                ConversationStores::open(data_dir.join(CONVERSATION_DATABASES), conversations::MAX_WRITERS).await?;
            let objects = objects::open(data_dir, s3).await?;
            Ok::<_, StorageError>((conversations, BlobStores::new(objects.clone()), ChangeStore::new(objects)))
        }
        .await;
        match rest {
            Ok((conversations, blobs, changes)) => Ok(Self {
                control,
                conversations,
                blobs,
                changes,
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
        native: NativeCatalog,
        user_streams: &BTreeMap<String, NativeOperation>,
        cloud: CloudServices,
        lifecycle: LifecycleTuning,
        expose_domain: Option<ExposeDomain>,
        expose_tuning: ExposeTuning,
    ) -> Result<Self, StartError> {
        let Storage {
            control,
            conversations,
            blobs,
            changes,
        } = storage;
        let hasher = PasswordHasher::new().await?;
        let clock = providers.clock.clone();
        let edge = tokio::runtime::Handle::current();
        // The shared services' own client; each shard thread builds its own.
        let http = reqwest::Client::builder().build().map_err(StartError::Http)?;
        let vault = Vault::new(control.clone(), secret.vault_key(), mode);
        let models_dev = ModelsDevClient::new(http.clone(), providers.models_dev_url, providers.clock.clone());
        let claude_releases = ClaudeReleases::new(&providers.claude_releases, edge.clone()).map_err(StartError::Http)?;
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
            changes,
            vault,
            assembly,
            claude_releases,
            cli_installs: CliInstalls::default(),
            operations,
            logins,
            claims: PendingClaims::new(runners.claims_per_minute),
            runners,
            conversation_tuning,
            user_streams: UserStreams::new(user_streams, &native),
            native,
            cloud,
            lifecycle,
            expose_domain,
            expose_tuning,
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
        Self::start_for_tests_with_lifecycle(data, LifecycleTuning::default()).await
    }

    /// The services over storage in `data` whose idle watches follow
    /// `lifecycle`, for unit tests of the idle release.
    #[cfg(test)]
    pub(crate) async fn start_for_tests_with_lifecycle(
        data: &std::path::Path,
        lifecycle: LifecycleTuning,
    ) -> Arc<Self> {
        let clock: Arc<dyn demi_core::Clock> = Arc::new(demi_core::SystemClock);
        let storage = Storage::open(data, clock.clone(), None).await.unwrap();
        let secret = InstanceSecret::load_or_create(data).await.unwrap();
        let providers = ProviderSetup {
            families: FamilyRegistry::builtin(),
            models_dev_url: ModelsDevClient::DEFAULT_URL.parse().unwrap(),
            claude_releases: crate::llm::claude_releases::DEFAULT_RELEASES_URL.parse().unwrap(),
            logins: LoginTiming::default(),
            clock,
        };
        // No manager listens there: a Cloud a unit test uses fails to start.
        let (machines, _deaths) = MachinesClient::new(data.join("machines.sock"));
        let services = Self::start(
            InstanceMode::Shared,
            storage,
            &secret,
            None,
            providers,
            RunnerTuning::default(),
            ConversationTuning::default(),
            NativeCatalog::unpublished(),
            &BTreeMap::new(),
            CloudServices::new(machines, crate::config::CloudTuning::default()),
            lifecycle,
            None,
            ExposeTuning::default(),
        )
        .await;
        Arc::new(services.unwrap())
    }
}

/// A running backend.
pub struct Backend {
    local_addr: SocketAddr,
    storage: Storage,
    services: Arc<Services>,
    shards: ShardPool,
    edge: Edge,
    /// Routes the machine manager's death events to their owners' shards.
    deaths: AbortOnDropHandle<()>,
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
    #[error("DEMI_CHANGE_STORE_CONFIG cannot be used: {0}")]
    ChangeStore(S3ConfigError),
    #[error("password hashing cannot start: {0}")]
    Hashing(#[from] HashError),
    #[error("the HTTP client cannot start: {0}")]
    Http(reqwest::Error),
    #[error("the shard threads cannot start: {0}")]
    Shards(io::Error),
    #[error("the backend cannot listen on {address}: {source}")]
    Listen { address: SocketAddr, source: io::Error },
    /// The machine manager did not reconcile, or an interrupted reset could
    /// not be finished.
    #[error("the Clouds cannot be recovered: {0}")]
    Cloud(String),
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
    /// A user's Cloud could not be saved; the manager's reconcile saves it.
    #[error("a Cloud was not saved: {0}")]
    Cloud(String),
    /// The machine manager did not reconcile at close.
    #[error("the machine manager did not reconcile: {0}")]
    Machines(String),
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
        let s3 = match &config.change_store {
            Some(path) => Some(S3Config::read(path).await.map_err(StartError::ChangeStore)?),
            None => None,
        };
        let storage = Storage::open(&data_dir, config.clock.clone(), s3.as_ref()).await?;
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
            claude_releases: config.claude_releases,
            logins: config.logins,
            clock: config.clock,
        };
        let (machines, deaths) = MachinesClient::new(config.machines_socket);
        let services = Services::start(
            config.mode,
            storage.clone(),
            secret,
            config.account_mail,
            providers,
            config.runners,
            config.conversations,
            config.native,
            &config.user_streams,
            CloudServices::new(machines, config.cloud),
            config.lifecycle,
            config.expose_domain,
            config.exposes,
        );
        let services = Arc::new(services.await?);
        let shards = match ShardPool::start(config.shards, services.clone()).await {
            Ok(shards) => shards,
            Err(error) => {
                services.close_providers().await;
                return Err(StartError::Shards(error));
            }
        };
        let deaths = AbortOnDropHandle::new(tokio::spawn(crate::managed::route_deaths(
            deaths,
            services.clone(),
            shards.shards(),
        )));
        // Before the backend serves, the machine manager settles what an
        // earlier backend left, which stops every Cloud and so ends their
        // exposes, and resets it left unfinished commit their disks.
        if let Err(error) = recover_resets(&services).await {
            shards.close().await;
            services.close_providers().await;
            return Err(StartError::Cloud(error.to_string()));
        }
        // Fork destinations whose root committed before their publication are
        // published before the backend serves.
        if let Err(error) = crate::conversation::recover_forks(&services.control, &services.conversations).await {
            shards.close().await;
            services.close_providers().await;
            return Err(StartError::Storage(error));
        }
        let state = AppState {
            services: services.clone(),
            shards: shards.shards(),
            site: Arc::new(Site {
                public_url: config.public_url,
                runner_releases: config.runner_releases,
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
            deaths,
        })
    }

    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Holds every commit of a conversation's checkpoint from now on, until
    /// the hold is released.
    #[cfg(feature = "testing")]
    pub fn hold_commits(&self) -> crate::CommitHold {
        self.services.conversations.hold_commits()
    }

    /// Shuts the backend down. The listener closes first, so no new work
    /// starts and a new request on an open connection answers 503
    /// `backend_closing`; every step runs even when an earlier one fails, and
    /// the failures are reported together.
    pub async fn close(self) -> Result<(), ShutdownErrors> {
        let mut failures = Vec::new();
        self.edge.stop_accepting();
        self.services.logins.close().await;
        failures.extend(self.shards.close().await.into_iter().map(ShutdownError::Cloud));
        self.services.claims.close();
        // No shard is left to route a death to.
        drop(self.deaths);
        if let Err(error) = self.services.cloud.machines.close().await {
            failures.push(ShutdownError::Machines(error.to_string()));
        }
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
