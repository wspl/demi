//! The services every core of a process shares.
//!
//! The server gives one set to all of its users' cores, and a program's
//! process one set to the cores it opens. They span the cores, such as the
//! vault over each core's store, the model catalogs and the accounts'
//! rotation, and both start them with [`CoreServices::start`].

use std::sync::Arc;

use bytes::Bytes;
use demi_core_blobs::blobs::{BlobStores, UserBlobs};
use demi_core_store::conversations::ConversationBlobs;
use demi_core_store::schema::ReadBlob;
use demi_shared_types::BlobRef;
use demi_core_host_access::stream::UserStreams;
use demi_core_store::StorageError;
use demi_core_sync::SyncRegistry;
use demi_plugin_host::{Registry, RegistryError};
use demi_plugin_interface::PluginFactory;
use demi_provider_registry::catalog_cache::ModelCatalogCache;
use demi_provider_registry::families::FamilyRegistry;
use demi_provider_registry::quotas::AccountQuotas;
use demi_provider_registry::registry::ProviderRegistry;
use demi_provider_registry::shutdowns::ModelShutdowns;
use demi_provider_registry::store::RegistryStore;
use demi_provider_registry::vendors::VendorCatalog;
use demi_provider_claude_code::releases::ClaudeReleases;
use demi_core_providers::vault::entries::{Vault, VaultStores};
use demi_core_providers::vault::logins::{LoginFlows, LoginTiming};
use demi_core_providers::vault::operations::ProviderOperations;
use demi_core_hosts::devices::Returning;
use demi_core_hosts::packages::Packages;
use demi_core_hosts::public_url::PublicUrl;
use demi_runner_protocol::direct::StunUrl;
use demi_provider_common::media_catalog::{self, MediaCatalog};
use demi_provider_common::models_dev::ModelsDevClient;
use demi_shared_types::Clock;
use demi_web_api_protocol::settings::InstanceMode;
use url::Url;

use crate::conversation::claude_cli::{CliInstalls, MachineSlot};
use crate::tuning::{ConversationTuning, LifecycleTuning, MediaTuning, RunnerTuning};

/// What the core's services start with, besides where the cores' stores
/// and blobs are.
pub struct CoreSettings {
    /// Where the process is reached from outside, once it is known.
    pub public_url: PublicUrl,
    pub clock: Arc<dyn Clock>,
    pub families: FamilyRegistry,
    /// The plugins, in their order of registration.
    pub plugins: Vec<Box<dyn PluginFactory>>,
    /// The command packages the cores' runners run.
    pub packages: Arc<dyn Packages>,
    pub models_dev_url: Url,
    /// Where LiteLLM's model table is read.
    pub model_shutdowns_url: Url,
    /// The media catalog of the build's branch; none keeps the release's
    /// copy (`crates/provider-common/src/media_catalog.rs`).
    pub media_catalog_url: Option<Url>,
    /// The media catalog the release carries, or a test's.
    pub media_catalog: Arc<media_catalog::Catalog>,
    /// The Claude Code distribution.
    pub claude_releases: Url,
    /// Where the Claude Code machine is, which the core that runs it puts
    /// there as it starts and the Claude Code family reads too.
    pub claude_machine: MachineSlot,
    pub logins: LoginTiming,
    pub runners: RunnerTuning,
    pub conversations: ConversationTuning,
    pub media: MediaTuning,
    pub lifecycle: LifecycleTuning,
    pub stun: Vec<StunUrl>,
}

/// The blobs a conversation database's migration reads: those of its core's
/// namespace.
pub struct MigrationBlobs {
    blobs: UserBlobs,
    /// The runtime the reads run on.
    runtime: tokio::runtime::Handle,
}

impl MigrationBlobs {
    /// The reads of `blobs`, on `runtime`: a multi-threaded runtime, whose
    /// own threads drive the reads while the migration's thread waits.
    pub fn new(blobs: UserBlobs, runtime: tokio::runtime::Handle) -> Arc<ConversationBlobs> {
        Arc::new(Self { blobs, runtime })
    }
}

impl ReadBlob for MigrationBlobs {
    fn read(&self, blob: &BlobRef) -> Result<Option<Bytes>, String> {
        // A migration runs on its database's thread, outside the runtime,
        // and its transaction waits for the read: it blocks on it there.
        self.runtime
            .block_on(self.blobs.get(blob))
            .map_err(|error| error.to_string())
    }
}

/// Why the core's services did not start.
#[derive(Debug, thiserror::Error)]
pub enum CoreServicesError {
    #[error("the HTTP client cannot start: {0}")]
    Http(reqwest::Error),
    #[error("the plugins cannot start: {0}")]
    Plugins(#[from] RegistryError),
    #[error("the provider entries cannot be read: {0}")]
    Storage(#[from] StorageError),
}

/// The services every core of this process shares: one set for all of a
/// server's users' cores, one set for the cores a program's process opens.
pub struct CoreServices {
    /// Where runners and Cloud guests reach this backend, and where a
    /// borrowing server reaches an entry this one lends.
    pub public_url: PublicUrl,
    /// The wall clock the backend reads times from.
    pub clock: Arc<dyn Clock>,
    pub blobs: BlobStores,
    pub vault: Vault,
    pub provider_registry: Arc<ProviderRegistry>,
    /// The catalog of image and video models (`crates/provider-common/src/media_catalog.rs`).
    pub media: MediaCatalog,
    /// The vendor's Claude Code releases, which the CLI on the Claude Code machine follows.
    pub claude_releases: ClaudeReleases,
    /// The outcome of the CLI installs no conversation asked for.
    pub cli_installs: CliInstalls,
    /// The Claude Code machine, set as the cores start.
    pub claude_machine: MachineSlot,
    pub operations: Arc<ProviderOperations>,
    pub logins: Arc<LoginFlows>,
    /// The calls the schedules' URLs took this minute.
    pub hook_limits: crate::schedules::HookLimits,
    pub runners: RunnerTuning,
    pub conversation_tuning: ConversationTuning,
    /// How a video's job is waited for.
    pub media_tuning: MediaTuning,
    /// The same packages as the cores run them, at this backend's address.
    pub packages: Arc<dyn Packages>,
    /// The backend's plugins, checked at startup (`crates/plugin-host/src/lib.rs`).
    pub plugins: Arc<Registry>,
    /// The user streams a page may open.
    pub user_streams: UserStreams,
    /// The STUN servers pages and runners ask for their public address
    /// (`crates/runner-direct/src/lib.rs`).
    pub stun: Vec<StunUrl>,
    /// This run of the backend, chosen when the services start: the
    /// revisions counted in memory compare only within one run.
    pub run: String,
    /// When a conversation's Host resources are reclaimed.
    pub lifecycle: LifecycleTuning,
    /// Each user's open synchronization channels, which every change a
    /// page shows marks.
    pub sync: SyncRegistry,
    /// The devices the backend's last shutdown disconnected, which Host
    /// operations wait for in the grace after the start
    /// (`crates/runner-client/src/device_jobs.rs`).
    pub returning: Arc<Returning>,
}

impl CoreServices {
    /// Starts the services on the calling runtime, which the ones that spawn
    /// work of their own run it on, over the cores' stores `stores` and
    /// their blobs `blobs`, in the instance mode `mode`.
    /// `shared_subscriptions` says whether a shared instance's users infer
    /// with the master's subscriptions.
    pub async fn start(
        settings: CoreSettings,
        stores: Arc<dyn VaultStores>,
        mode: InstanceMode,
        shared_subscriptions: bool,
        blobs: BlobStores,
    ) -> Result<Self, CoreServicesError> {
        let CoreSettings {
            public_url,
            clock,
            families,
            plugins,
            packages,
            models_dev_url,
            model_shutdowns_url,
            media_catalog_url,
            media_catalog,
            claude_releases,
            claude_machine,
            logins,
            runners,
            conversations,
            media: media_tuning,
            lifecycle,
            stun,
        } = settings;
        let plugins = Registry::new(plugins, |operation| {
            packages.serves(&operation.package, &[operation.operation.as_str()])
        })?;
        let edge = tokio::runtime::Handle::current();
        // The shared services' own client; each core's thread builds its own.
        let http = reqwest::Client::builder()
            .build()
            .map_err(CoreServicesError::Http)?;
        let sync = SyncRegistry::default();
        let vault = Vault::new(stores, mode, shared_subscriptions, sync.clone()).await?;
        let models_dev = ModelsDevClient::new(
            http.clone(),
            models_dev_url,
            clock.clone(),
            &families.effort_vendors(),
        );
        let shutdowns = ModelShutdowns::new(http.clone(), model_shutdowns_url, clock.clone());
        shutdowns.start(&edge);
        let media = MediaCatalog::new(http.clone(), media_catalog_url, media_catalog);
        media.start(&edge);
        // A new catalog describes every core's media models anew: their
        // pages read them again. The task ends with the catalog, whose last
        // owner drops the sender.
        let mut catalogs = media.subscribe();
        let announced = sync.clone();
        edge.spawn(async move {
            while catalogs.changed().await.is_ok() {
                announced.mark_everyone(&demi_core_sync::Part::Providers);
            }
        });
        let claude_releases = ClaudeReleases::new(&claude_releases, edge.clone(), clock.clone())
            .map_err(CoreServicesError::Http)?;
        let store: Arc<dyn RegistryStore> = Arc::new(vault.clone());
        let provider_registry = Arc::new(ProviderRegistry::new(
            store.clone(),
            families,
            AccountQuotas::new(store.clone(), edge.clone(), clock.clone()),
            ModelCatalogCache::new(store, clock.clone(), edge),
            VendorCatalog::new(models_dev, shutdowns),
            http,
            clock.clone(),
            media.clone(),
        ));
        // A source's new model list changes the catalog of every core that
        // infers with its entry: their pages hear of it. The task ends with
        // the cache, whose last owner drops the sender.
        let mut models = provider_registry.catalogs().subscribe();
        let announced = sync.clone();
        tokio::spawn(async move {
            while models.changed().await.is_ok() {
                announced.mark_everyone(&demi_core_sync::Part::Models);
            }
        });
        let operations = Arc::new(ProviderOperations::default());
        let logins = LoginFlows::new(
            vault.clone(),
            provider_registry.clone(),
            operations.clone(),
            logins,
            clock.clone(),
            sync.clone(),
        );
        Ok(Self {
            public_url,
            clock,
            blobs,
            vault,
            provider_registry,
            media,
            claude_releases,
            cli_installs: CliInstalls::default(),
            claude_machine,
            operations,
            logins,
            hook_limits: crate::schedules::HookLimits::default(),
            runners,
            conversation_tuning: conversations,
            media_tuning,
            user_streams: UserStreams::new(
                plugins
                    .streams()
                    .map(|stream| (stream.name.as_str(), &stream.operation)),
                packages.as_ref(),
            ),
            packages,
            plugins: Arc::new(plugins),
            stun,
            run: uuid::Uuid::new_v4().to_string(),
            lifecycle,
            sync,
            returning: Arc::new(Returning::default()),
        })
    }

    /// Cancels the logins, drains the catalog refreshes and quota
    /// writes, before storage closes.
    pub async fn close_providers(&self) {
        self.logins.close().await;
        self.provider_registry.close().await;
        self.media.close();
    }
}
