//! The services every shard is given (`backend.md` § Runtime model): the
//! ones that span users, or that are needed before the user is known, which
//! the edge and every shard share, over the storage they are opened on.

use std::path::Path;
use std::sync::Arc;

use demi_backend_accounts::email_change::{AccountMail, CodeKey, EmailChanges};
use demi_backend_accounts::login_limiter::LoginLimiter;
use demi_backend_accounts::passwords::{HashError, PasswordHasher};
use demi_backend_accounts::sessions::WebSessions;
use demi_backend_blobs::blobs::BlobStores;
use demi_backend_cloud::CloudServices;
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::conversations::{self, ConversationStores};
use demi_backend_expose::domain::ExposeDomain;
use demi_backend_host_access::stream::UserStreams;
use demi_backend_page_sync::SyncRegistry;
use demi_backend_plugins::{Registry, RegistryError};
use demi_backend_providers::llm::assembly::ProviderAssembly;
use demi_backend_providers::llm::catalog_cache::ModelCatalogCache;
use demi_backend_providers::llm::claude_releases::ClaudeReleases;
use demi_backend_providers::llm::families::FamilyRegistry;
use demi_backend_providers::llm::vendors::VendorCatalog;
use demi_backend_providers::vault::entries::Vault;
use demi_backend_providers::vault::logins::{LoginFlows, LoginTiming};
use demi_backend_providers::vault::operations::ProviderOperations;
use demi_backend_providers::vault::quotas::AccountQuotas;
use demi_backend_providers::vault::seal::VaultKey;
use demi_backend_runners::claims::PendingClaims;
use demi_backend_runners::native::NativeCatalog;
use demi_backend_runners::public_url::PublicUrl;
use demi_plugin_interface::PluginFactory;
use demi_provider_common::models_dev::ModelsDevClient;
use demi_shared_types::Clock;
use demi_web_api_protocol::settings::InstanceMode;
use object_store::ObjectStore;

use crate::conversation::claude_cli::CliInstalls;
use crate::tuning::{ConversationTuning, ExposeTuning, LifecycleTuning, PageTuning, RunnerTuning};

const CONTROL_DATABASE: &str = "control.sqlite";
const CONVERSATION_DATABASES: &str = "conversations";

/// The services that span users, or that are needed before the user is
/// known. The edge and every shard share them.
pub struct Services {
    pub mode: InstanceMode,
    /// The wall clock the backend reads times from.
    pub clock: Arc<dyn Clock>,
    pub control: ControlService,
    pub conversations: ConversationStores,
    pub blobs: BlobStores,
    pub hasher: PasswordHasher,
    pub sessions: WebSessions,
    pub limiter: LoginLimiter,
    pub email: EmailChanges,
    pub vault: Vault,
    pub assembly: Arc<ProviderAssembly>,
    /// The vendor's Claude Code releases, which the CLI on each Cloud follows.
    pub claude_releases: ClaudeReleases,
    /// The outcome of the CLI installs no conversation asked for.
    pub cli_installs: CliInstalls,
    pub operations: Arc<ProviderOperations>,
    pub logins: Arc<LoginFlows>,
    /// Runners waiting to be paired, which have no user yet.
    pub claims: PendingClaims,
    pub runners: RunnerTuning,
    pub conversation_tuning: ConversationTuning,
    /// How the sockets to a page are timed.
    pub pages: PageTuning,
    /// The command packages each shard's catalog is built from.
    pub native: NativeCatalog,
    /// The backend's plugins, checked at startup (`plugins.md` § The plugin
    /// host).
    pub plugins: Arc<Registry>,
    /// The user streams a page may open.
    pub user_streams: UserStreams,
    /// The machine manager's client, the Cloud capacity across users and
    /// the Cloud's settings.
    pub cloud: CloudServices,
    /// Where runners, Cloud guests and expose visitors reach this backend.
    pub public_url: PublicUrl,
    /// The build of the web app the backend serves, if it serves one.
    pub web_build: Option<String>,
    /// This run of the backend, chosen when the services start: the
    /// revisions counted in memory compare only within one run
    /// (`web-api.md` § Revisions counted in memory).
    pub run: String,
    /// When a conversation's Host resources are reclaimed.
    pub lifecycle: LifecycleTuning,
    /// The domain of expose hostnames; without it, exposes are unavailable.
    pub expose_domain: Option<ExposeDomain>,
    /// How the public relay treats its connections.
    pub expose_tuning: ExposeTuning,
    /// Each user's open synchronization channels, which every change a
    /// page shows marks.
    pub sync: SyncRegistry,
    /// The step a test holds runners' hellos at (`Backend::hold_hellos`).
    #[cfg(feature = "testing")]
    pub hellos: crate::holds::StepHolds<crate::holds::HelloStep>,
    /// The step a test holds the synchronization channels at
    /// (`Backend::hold_sync`).
    #[cfg(feature = "testing")]
    pub syncs: crate::holds::StepHolds<crate::sync::SyncStep>,
}

/// What the provider services start with.
pub struct ProviderSetup {
    pub families: FamilyRegistry,
    pub models_dev_url: url::Url,
    /// The Claude Code distribution.
    pub claude_releases: url::Url,
    pub logins: LoginTiming,
    pub clock: Arc<dyn Clock>,
}

/// The keys the services seal and hash under, which the executable derives
/// from its instance secret.
pub struct ServiceKeys {
    /// The key provider credentials are sealed under.
    pub vault: VaultKey,
    /// The key email-change codes are hashed under.
    pub email_codes: CodeKey,
}

/// What the services are configured with besides storage, the keys and the
/// providers.
pub struct ServiceSettings {
    pub mode: InstanceMode,
    /// The build of the web app the backend serves, if it serves one.
    pub web_build: Option<String>,
    pub mail: Option<Arc<dyn AccountMail>>,
    pub runners: RunnerTuning,
    pub conversations: ConversationTuning,
    pub pages: PageTuning,
    pub native: NativeCatalog,
    /// The plugins, in their order of registration.
    pub plugins: Vec<Box<dyn PluginFactory>>,
    pub cloud: CloudServices,
    pub lifecycle: LifecycleTuning,
    pub expose_domain: Option<ExposeDomain>,
    pub exposes: ExposeTuning,
}

/// Why the services did not start.
#[derive(Debug, thiserror::Error)]
pub enum ServicesError {
    #[error("password hashing cannot start: {0}")]
    Hashing(#[from] HashError),
    #[error("the HTTP client cannot start: {0}")]
    Http(reqwest::Error),
    #[error("the plugins cannot start: {0}")]
    Plugins(#[from] RegistryError),
}

/// The databases and the object store the services run on.
#[derive(Clone)]
pub struct Storage {
    pub control: ControlService,
    pub conversations: ConversationStores,
    pub blobs: BlobStores,
}

/// A database that did not close.
#[derive(Debug, thiserror::Error)]
pub enum CloseError {
    #[error("a conversation database did not close: {0}")]
    Conversation(StorageError),
    #[error("the control database did not close: {0}")]
    Control(StorageError),
}

impl Storage {
    /// The databases in `data_dir`, beside the object store `objects`.
    pub async fn open(
        data_dir: &Path,
        clock: Arc<dyn Clock>,
        objects: Arc<dyn ObjectStore>,
    ) -> Result<Self, StorageError> {
        let control = ControlService::open(&data_dir.join(CONTROL_DATABASE), clock.clone()).await?;
        let rest = async {
            let conversations = ConversationStores::open(
                data_dir.join(CONVERSATION_DATABASES),
                conversations::MAX_WRITERS,
            )
            .await?;
            let blobs = BlobStores::new(objects, clock);
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
    pub async fn close(&self) -> Vec<CloseError> {
        let mut failures: Vec<CloseError> = self
            .conversations
            .close()
            .await
            .into_iter()
            .map(CloseError::Conversation)
            .collect();
        if let Err(error) = self.control.close().await {
            failures.push(CloseError::Control(error));
        }
        failures
    }
}

impl Services {
    /// Starts the services on the edge's runtime, which the ones that spawn
    /// work of their own run it on.
    pub async fn start(
        storage: Storage,
        keys: ServiceKeys,
        providers: ProviderSetup,
        settings: ServiceSettings,
    ) -> Result<Self, ServicesError> {
        let Storage {
            control,
            conversations,
            blobs,
        } = storage;
        let native = &settings.native;
        let plugins = Registry::new(settings.plugins, |operation| {
            native.serves(&operation.package, &[operation.operation.as_str()])
        })?;
        let hasher = PasswordHasher::new().await?;
        let clock = providers.clock.clone();
        let edge = tokio::runtime::Handle::current();
        // The shared services' own client; each shard thread builds its own.
        let http = reqwest::Client::builder()
            .build()
            .map_err(ServicesError::Http)?;
        let sync = SyncRegistry::default();
        let vault = Vault::new(control.clone(), keys.vault, settings.mode, sync.clone());
        let models_dev = ModelsDevClient::new(
            http.clone(),
            providers.models_dev_url,
            providers.clock.clone(),
        );
        let claude_releases = ClaudeReleases::new(&providers.claude_releases, edge.clone())
            .map_err(ServicesError::Http)?;
        let assembly = Arc::new(ProviderAssembly::new(
            vault.clone(),
            providers.families,
            AccountQuotas::new(vault.clone(), edge.clone()),
            ModelCatalogCache::new(control.clone(), providers.clock.clone(), edge),
            VendorCatalog::new(models_dev),
            http,
            providers.clock,
        ));
        let operations = Arc::new(ProviderOperations::default());
        let logins = LoginFlows::new(
            assembly.clone(),
            operations.clone(),
            providers.logins,
            clock.clone(),
        );
        Ok(Self {
            mode: settings.mode,
            clock,
            sessions: WebSessions::new(control.clone()),
            limiter: LoginLimiter::new(),
            email: EmailChanges::new(
                control.clone(),
                hasher.clone(),
                settings.mail,
                keys.email_codes,
            ),
            hasher,
            control,
            conversations,
            blobs,
            vault,
            assembly,
            claude_releases,
            cli_installs: CliInstalls::default(),
            operations,
            logins,
            claims: PendingClaims::new(settings.runners.claims_per_minute),
            runners: settings.runners,
            conversation_tuning: settings.conversations,
            pages: settings.pages,
            user_streams: UserStreams::new(
                plugins
                    .streams()
                    .map(|stream| (stream.name.as_str(), &stream.operation)),
                &settings.native,
            ),
            native: settings.native,
            plugins: Arc::new(plugins),
            cloud: settings.cloud,
            public_url: PublicUrl::default(),
            web_build: settings.web_build,
            run: uuid::Uuid::new_v4().to_string(),
            lifecycle: settings.lifecycle,
            expose_domain: settings.expose_domain,
            expose_tuning: settings.exposes,
            sync,
            #[cfg(feature = "testing")]
            hellos: crate::holds::StepHolds::default(),
            #[cfg(feature = "testing")]
            syncs: crate::holds::StepHolds::default(),
        })
    }

    /// Cancels the logins, and drains the catalog refreshes and quota
    /// writes, before storage closes.
    pub async fn close_providers(&self) {
        self.logins.close().await;
        self.assembly.close().await;
    }

    /// The services over storage in `data`, for tests: no provider family is
    /// registered, and no machine manager listens.
    #[cfg(any(test, feature = "testing"))]
    pub async fn start_for_tests(data: &Path) -> Arc<Self> {
        Self::start_for_tests_with_lifecycle(data, LifecycleTuning::default()).await
    }

    /// The services over storage in `data` whose idle watches follow
    /// `lifecycle`, for tests of the idle release.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) async fn start_for_tests_with_lifecycle(
        data: &Path,
        lifecycle: LifecycleTuning,
    ) -> Arc<Self> {
        let clock: Arc<dyn Clock> = Arc::new(demi_shared_types::SystemClock);
        let objects = demi_backend_blobs::store::open(data, &demi_backend_blobs::store::Storage::Local)
            .await
            .unwrap()
            .store;
        let storage = Storage::open(data, clock.clone(), objects).await.unwrap();
        let keys = ServiceKeys {
            vault: VaultKey::new(rand::random()),
            email_codes: CodeKey::new(rand::random()),
        };
        let providers = ProviderSetup {
            families: FamilyRegistry::default(),
            models_dev_url: ModelsDevClient::DEFAULT_URL.parse().unwrap(),
            claude_releases: demi_backend_providers::llm::claude_releases::DEFAULT_RELEASES_URL
                .parse()
                .unwrap(),
            logins: LoginTiming::default(),
            clock,
        };
        // No manager listens there: a Cloud a test uses fails to start.
        let (machines, _deaths) =
            demi_backend_cloud::client::MachinesClient::new(data.join("machines.sock"));
        let settings = ServiceSettings {
            mode: InstanceMode::Shared,
            web_build: None,
            mail: None,
            runners: RunnerTuning::default(),
            conversations: ConversationTuning::default(),
            pages: PageTuning::default(),
            native: NativeCatalog::unpublished(),
            plugins: Vec::new(),
            cloud: CloudServices::new(machines, demi_backend_cloud::tuning::CloudTuning::default()),
            lifecycle,
            expose_domain: None,
            exposes: ExposeTuning::default(),
        };
        let services = Self::start(storage, keys, providers, settings).await;
        Arc::new(services.unwrap())
    }
}
