//! The services every core is given (`crates/backend/src/lib.rs`).
//!
//! They span users, or are needed before the user is known. The edge and every
//! core share them, over the storage they are opened on.

use std::path::Path;
use std::sync::Arc;

use demi_backend_accounts::email_change::{AccountMail, CodeKey, EmailChanges};
use demi_backend_accounts::login_limiter::LoginLimiter;
use demi_backend_accounts::passwords::{HashError, PasswordHasher};
use demi_backend_accounts::push::{PublicPushTransport, PushKey, PushTransport, Pusher};
use demi_backend_accounts::sessions::WebSessions;
use demi_core_blobs::blobs::BlobStores;
use demi_backend_cloud::CloudServices;
use demi_core_store::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::core_stores::{CoreStores, UserBlobs as BlobsOfUser};
use demi_core_store::conversations::ConversationBlobs;
use demi_backend_database::server_log::ServerLog;
use demi_web_api_protocol::ids::UserId;
use demi_backend_database::schema::ControlSteps;
use demi_core_providers::vault::entries::{SeededAccount, VaultReseal, VaultStores};
use demi_core_store::store::CoreStore;
use demi_web_api_protocol::ids::ProviderId;
use demi_web_api_protocol::settings::InstanceMode;
use futures_util::future::BoxFuture;
use demi_core_store::seal::VaultKey;
use demi_backend_runners::claims::PendingClaims;
use demi_backend_runners::native::NativeCatalog;
use demi_backend_runners::install::RunnerReleases;
use demi_backend_runners::prefetch::Prefetches;
use demi_shared_types::Clock;
use object_store::ObjectStore;

use demi_core::services::{CoreServices, CoreServicesError, CoreSettings, MigrationBlobs};

const CONTROL_DATABASE: &str = "control.sqlite";

/// The services that span users, or that are needed before the user is
/// known. The edge and every core share them.
pub struct Services {
    /// What the users' cores share.
    pub core: Arc<CoreServices>,
    pub control: ControlService,
    /// Each user's core store.
    pub stores: CoreStores,
    pub hasher: PasswordHasher,
    pub sessions: WebSessions,
    pub limiter: LoginLimiter,
    pub email: EmailChanges,
    /// The users' push subscriptions, and the sending to them.
    pub push: Pusher,
    /// The account the master account's entry of its family holds from each
    /// start and from setup, a development backend's only (`crates/xtask/src/dev/mod.rs`).
    pub master_account: Option<SeededAccount>,
    /// Runners waiting to be paired, which have no user yet.
    pub claims: PendingClaims,
    /// The log, which the runners' and the pages' lines are stored in
    /// (`crates/backend-database/src/server_log/mod.rs`).
    pub log: ServerLog,
    /// The command packages each core's catalog is built from.
    pub native: NativeCatalog,
    /// The machine manager's client, the Cloud capacity across users and
    /// the Cloud's settings.
    pub cloud: Arc<CloudServices>,
    /// The sessions and media calls other servers borrow this one's entries
    /// in, by share key, which revoking a key ends.
    pub lent: crate::provider_sharing::LentSessions,
    /// The build of the web app the backend serves, if it serves one.
    pub web_build: Option<String>,
    /// The runner releases paired devices follow.
    pub runner_releases: RunnerReleases,
    /// The runner release an upgrade announced as next, which the
    /// connected paired devices download ahead (`crates/runner/src/update.rs`).
    pub prefetches: Prefetches,
    /// The server's updates, Settings › Updates.
    pub upgrades: Arc<crate::upgrades::Upgrades>,
}

impl std::ops::Deref for Services {
    type Target = CoreServices;

    fn deref(&self) -> &CoreServices {
        &self.core
    }
}

/// The keys the services seal and hash under, which the executable derives
/// from its instance secret.
pub struct ServiceKeys {
    /// The key email-change codes are hashed under.
    pub email_codes: CodeKey,
    /// The VAPID key push messages are signed with.
    pub push: PushKey,
}

/// What the server's own services are configured with besides storage and
/// the keys.
pub struct ServiceSettings {
    pub shared_subscriptions: bool,
    /// The account the master account's entry of its family holds.
    pub master_account: Option<SeededAccount>,
    pub log: ServerLog,
    /// The build of the web app the backend serves, if it serves one.
    pub web_build: Option<String>,
    pub runner_releases: RunnerReleases,
    pub mail: Option<Arc<dyn AccountMail>>,
    /// The command packages the server's release published.
    pub native: NativeCatalog,
    pub cloud: CloudServices,
    /// The file an administrator's update is requested in; none on a
    /// server that offers no updates.
    pub upgrade_request: Option<std::path::PathBuf>,
    /// The channel Settings › Updates starts on until an administrator
    /// chooses another.
    pub upgrade_channel: demi_web_api_protocol::server_upgrade::UpgradeChannel,
    /// The backend's update state, which says when it stops for an update.
    pub update: Arc<demi_backend_database::update::Update>,
}

/// Why the services did not start.
#[derive(Debug, thiserror::Error)]
pub enum ServicesError {
    #[error("password hashing cannot start: {0}")]
    Hashing(#[from] HashError),
    #[error("the HTTP client cannot start: {0}")]
    Http(reqwest::Error),
    #[error(transparent)]
    Core(#[from] CoreServicesError),
    #[error("the server's records cannot be read: {0}")]
    Storage(#[from] StorageError),
}

/// The databases and the object store the services run on.
#[derive(Clone)]
pub struct Storage {
    pub control: ControlService,
    pub stores: CoreStores,
    pub blobs: BlobStores,
}

/// What the control database's migrations ask: the data directory, and the
/// vault's key that seals a value again.
struct ServerReseal {
    vault: VaultReseal,
    data_directory: std::path::PathBuf,
}

impl ControlSteps for ServerReseal {
    fn data_directory(&self) -> &Path {
        &self.data_directory
    }

    fn provider_config(&self, id: &ProviderId, family: &str, sealed: &[u8]) -> Result<Vec<u8>, String> {
        self.vault.provider_config(id, family, sealed)
    }

    fn schedule_secret(&self, id: &str, sealed: &[u8]) -> Result<Vec<u8>, String> {
        self.vault.schedule_secret(id, sealed)
    }

    fn provider_endpoint_on_device(
        &self,
        id: &ProviderId,
        sealed: &[u8],
        owned: &dyn Fn(&str) -> bool,
    ) -> Result<Option<(String, Vec<u8>)>, String> {
        self.vault.provider_endpoint_on_device(id, sealed, owned)
    }

    fn provider_config_format_3(&self, id: &ProviderId, sealed: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.vault.provider_config_format_3(id, sealed)
    }
}

/// The users' cores, whose entries the vault keeps, with the control
/// database that names the users, the master and the mode.
struct ServerVaultStores {
    control: ControlService,
    stores: CoreStores,
}

impl VaultStores for ServerVaultStores {
    fn store_of<'a>(&'a self, user: &'a UserId) -> BoxFuture<'a, Result<CoreStore, StorageError>> {
        Box::pin(self.stores.of(user))
    }

    fn users(&self) -> BoxFuture<'_, Result<Vec<UserId>, StorageError>> {
        Box::pin(self.control.user_ids())
    }

    fn master(&self) -> BoxFuture<'_, Result<Option<UserId>, StorageError>> {
        Box::pin(self.control.master())
    }

    fn keep_mode(&self, mode: InstanceMode) -> BoxFuture<'_, Result<(), StorageError>> {
        Box::pin(self.control.set_instance_mode(mode))
    }
}

/// The blobs each user's conversation databases' migrations read.
struct UsersBlobs {
    blobs: BlobStores,
    /// The backend's runtime, which the reads run on.
    runtime: tokio::runtime::Handle,
}

impl BlobsOfUser for UsersBlobs {
    fn of(&self, user: &UserId) -> Arc<ConversationBlobs> {
        MigrationBlobs::new(self.blobs.for_user(user), self.runtime.clone())
    }
}

/// A database that did not close.
#[derive(Debug, thiserror::Error)]
pub enum CloseError {
    #[error("a user's database did not close: {0}")]
    Conversation(StorageError),
    #[error("the control database did not close: {0}")]
    Control(StorageError),
}

impl Storage {
    /// The bytes of the databases in `data_dir` that the next start
    /// migrates or converts; none when all have the current form.
    pub fn due(data_dir: &Path) -> Result<Option<demi_backend_database::update::Due>, StorageError> {
        demi_backend_database::update::due(&data_dir.join(CONTROL_DATABASE), data_dir)
    }

    /// The databases in `data_dir`, beside the object store `objects`, whose
    /// cores seal under `vault`, which an earlier control database's sealed
    /// values are migrated under too.
    pub async fn open(
        data_dir: &Path,
        clock: Arc<dyn Clock>,
        objects: Arc<dyn ObjectStore>,
        vault: VaultKey,
    ) -> Result<Self, StorageError> {
        let control = ControlService::open(
            &data_dir.join(CONTROL_DATABASE),
            clock.clone(),
            Arc::new(ServerReseal {
                vault: VaultReseal::new(vault.clone()),
                data_directory: data_dir.to_owned(),
            }),
        )
        .await?;
        let blobs = BlobStores::new(objects, clock.clone());
        let stores = CoreStores::new(
            data_dir.to_owned(),
            vault,
            clock.clone(),
            Arc::new(UsersBlobs {
                blobs: blobs.clone(),
                runtime: tokio::runtime::Handle::current(),
            }),
        );
        Ok(Self { control, stores, blobs })
    }

    /// Closes the users' core stores, then the control database, and
    /// answers every step that failed.
    pub async fn close(&self) -> Vec<CloseError> {
        let mut failures: Vec<CloseError> = self
            .stores
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
    /// work of their own run it on: the cores' over the users' stores, then
    /// the server's own.
    pub async fn start(
        storage: Storage,
        keys: ServiceKeys,
        core: CoreSettings,
        settings: ServiceSettings,
    ) -> Result<Self, ServicesError> {
        let Storage {
            control,
            stores,
            blobs,
        } = storage;
        let hasher = PasswordHasher::new().await?;
        let clock = core.clock.clone();
        let mode = control.instance_mode().await?;
        let core = CoreServices::start(
            core,
            Arc::new(ServerVaultStores {
                control: control.clone(),
                stores: stores.clone(),
            }),
            mode,
            settings.shared_subscriptions,
            blobs,
        )
        .await?;
        let cores: Vec<CoreStore> = stores.all(&control).await?.into_iter().map(|(_, store)| store).collect();
        core.returning.take(&cores).await?;
        // A data directory that predates the development sign-in, or holds
        // an earlier one, is brought to the current one before anything
        // infers with it.
        if let Some(account) = &settings.master_account
            && let Some(master) = control.master().await?
        {
            core.vault.seed(master, account, clock.now()).await?;
        }
        let upgrades = crate::upgrades::Upgrades::start(
            settings.upgrade_request.clone(),
            settings.upgrade_channel,
            control.clone(),
            core.sync.clone(),
            settings.update.clone(),
            clock,
        )
        .await?;
        let push_transport: Arc<dyn PushTransport> =
            Arc::new(PublicPushTransport::new().map_err(ServicesError::Http)?);
        let push = Pusher::new(control.clone(), keys.push, push_transport);
        let claims_per_minute = core.runners.claims_per_minute;
        Ok(Self {
            core: Arc::new(core),
            sessions: WebSessions::new(control.clone()),
            limiter: LoginLimiter::new(),
            email: EmailChanges::new(
                control.clone(),
                stores.clone(),
                hasher.clone(),
                settings.mail,
                keys.email_codes,
            ),
            push,
            hasher,
            claims: PendingClaims::new(control.clone(), claims_per_minute),
            control,
            stores,
            master_account: settings.master_account,
            log: settings.log,
            native: settings.native,
            cloud: Arc::new(settings.cloud),
            lent: Default::default(),
            web_build: settings.web_build,
            runner_releases: settings.runner_releases,
            prefetches: Prefetches::default(),
            upgrades,
        })
    }

    /// Tells the pages that show how Clouds are isolated: Settings › Cloud's,
    /// and Settings › Users', whose Cloud Storage applies as the isolation
    /// enforces it. The master's choice changes them, and so does what the
    /// machine manager enforces with each isolation.
    pub fn isolation_changed(&self) {
        self.sync.mark_everyone(&demi_core_sync::Part::CloudIsolation);
        self.sync.mark_everyone(&demi_core_sync::Part::Users);
    }

    /// Tells Settings › Cloud that a runtime's download moved on, which
    /// changes nothing Settings › Users shows.
    pub fn download_changed(&self) {
        self.sync.mark_everyone(&demi_core_sync::Part::CloudIsolation);
    }
}
