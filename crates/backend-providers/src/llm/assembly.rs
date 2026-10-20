//! Provider assembly
//! (`crates/backend-user-shard/src/conversation/providers.rs`): the provider of
//! each entry and account, built through the
//! entry's family from a fresh read of the entry and reused only while the
//! entry read is unchanged, so a lookup that completes after an edit cannot
//! keep later requests on a stale configuration.
//!
//! A provider stands for one entry and one credential, an API-key entry's key
//! or one subscription account. The entry's active account is only the default
//! for inference; a connection test, a usage probe and a refresh name the
//! account they mean. A provider never changes accounts, so a request's quota
//! observations land on the account that made it, and a request that outlives a
//! switch cannot credit the wrong account. Selecting another account builds a
//! new provider; a running request finishes with its own.

use demi_provider_common::media_catalog::MediaCatalog;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use demi_backend_database::StorageError;
use demi_provider_claude_code::{AccountMachine, Placement};
use demi_provider_common::credentials::{CredentialPool, SubscriptionAccounts};
use demi_provider_common::images::ImageMaker;
use demi_provider_common::media::{MediaMaker, MediaModelInfo};
use demi_provider_common::audio::AudioMaker;
use demi_provider_common::videos::VideoMaker;
use demi_provider_common::{DeviceRoute, Provider, ProviderHttp, ProviderRuntime, RuntimeEnv};
use demi_shared_types::{AudioShape, Clock, ImageShape, MediaKind, MediaShapes, Model, Rotation};
use demi_web_api_protocol::ids::{CredentialId, ProviderId};
use demi_provider_common::models_dev::ModelsDevError;
use demi_web_api_protocol::providers::{ModelKind, ProviderDto, SETUP_MESSAGE, Vendor};
use demi_web_api_protocol::text::EndpointUrl;

use super::catalog_cache::ModelCatalogCache;
use super::rotation::{AccountRotation, Candidate, Choice};
use super::families::{
    AccountBinding, ApiKeyArgs, CompatibleAudioArgs, CompatibleImagesArgs, CompatibleVideosArgs, FamilyArgs, FamilyCredential, FamilyError,
    FamilyRegistry, ProviderFamily, SubscriptionArgs,
};
use super::vendors::{VendorCatalog, sort_by_name};

/// The family of the vendors that make media and no text (`crates/backend/src/families.rs`).
pub const MEDIA_FAMILY: &str = "media";
use crate::vault::entries::{EntryCredential, ProviderEntry, Vault};
use crate::vault::quotas::AccountQuotas;

/// Why an entry's provider could not be built.
#[derive(Debug, thiserror::Error)]
pub enum AssemblyError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The entry names a family this backend does not register.
    #[error("the provider family {0} is not available")]
    UnknownFamily(String),
    #[error(transparent)]
    Family(#[from] FamilyError),
    /// The entry's provider says it runs a process, but its family builds
    /// no runtime over a placement.
    #[error("the provider family {0} cannot run its process")]
    NoProcessRuntime(String),
    /// An API-key entry without its key: nothing is sent until the user
    /// adds it.
    #[error("{SETUP_MESSAGE}")]
    Incomplete,
}

/// An entry's provider for one of its accounts, with the entry read it was
/// built from.
type Built = (ProviderEntry, Arc<dyn Provider>);

/// The provider of an entry and an account: the active one's, or another's
/// that an agent took under the entry's rotation.
type BuiltKey = (ProviderId, Option<CredentialId>);

/// How old a snapshot Balanced compares may be before it is read again.
const SNAPSHOT_FRESH_MS: i64 = 5 * 60 * 1000;

/// The account operations a login runs: its family's own over the machine
/// its sign-in runs on, or else the provider's.
pub enum LoginAccounts {
    Provider(Arc<dyn Provider>),
    Machine(Box<dyn SubscriptionAccounts>),
}

impl LoginAccounts {
    pub fn accounts(&self) -> Option<&dyn SubscriptionAccounts> {
        match self {
            Self::Provider(provider) => provider.accounts(),
            Self::Machine(accounts) => Some(&**accounts),
        }
    }
}

/// The providers of every entry, shared by the edge and the shards.
pub struct ProviderAssembly {
    vault: Vault,
    families: FamilyRegistry,
    quotas: Arc<AccountQuotas>,
    catalogs: ModelCatalogCache,
    vendors: VendorCatalog,
    /// The shared services' HTTP client, which providers make their own
    /// requests with.
    http: reqwest::Client,
    clock: Arc<dyn Clock>,
    /// The provider of each entry and account in use, with the entry it was
    /// built from. A `std` mutex: lookups never await under it.
    built: Mutex<HashMap<BuiltKey, Built>>,
    rotation: Arc<AccountRotation>,
    /// The Claude Code machine, set once the backend's shards started,
    /// which hold it.
    machine: OnceLock<Arc<dyn AccountMachine>>,
    /// The media catalog, whose time each entry's media carry.
    media: MediaCatalog,
    /// The way to the users' devices, set once the backend's shards
    /// started, which hold their connections (`crates/backend-user-shard/src/shard/exposed_ports.rs`).
    devices: OnceLock<Arc<dyn DeviceRoute>>,
}

impl ProviderAssembly {
    pub fn new(
        vault: Vault,
        families: FamilyRegistry,
        quotas: Arc<AccountQuotas>,
        catalogs: ModelCatalogCache,
        vendors: VendorCatalog,
        http: reqwest::Client,
        clock: Arc<dyn Clock>,
        media: MediaCatalog,
    ) -> Self {
        Self {
            media,
            vault,
            families,
            quotas,
            catalogs,
            vendors,
            http,
            clock,
            built: Mutex::default(),
            rotation: AccountRotation::new(),
            machine: OnceLock::new(),
            devices: OnceLock::new(),
        }
    }

    /// Sets the Claude Code machine, once, as the backend's shards start
    /// (`crates/backend-cloud/src/lib.rs`); a second setting is refused
    /// and the first kept.
    pub fn set_machine(&self, machine: Arc<dyn AccountMachine>) {
        if self.machine.set(machine).is_err() {
            tracing::error!("the Claude Code machine was set twice; the first is kept");
        }
    }

    /// Sets the way to the users' devices, once, as the backend's shards
    /// start; a second setting is refused and the first kept.
    pub fn set_devices(&self, route: Arc<dyn DeviceRoute>) {
        if self.devices.set(route).is_err() {
            tracing::error!("the way to the devices was set twice; the first is kept");
        }
    }

    /// The client `entry`'s requests go through on `client`, the shard's or
    /// the shared services': an address naming one of the entry owner's
    /// devices reaches that device's exposed port (`crates/backend-user-shard/src/shard/exposed_ports.rs`).
    pub fn http_for(&self, entry: &ProviderEntry, client: reqwest::Client) -> ProviderHttp {
        self.http_of(Some(entry.owner.as_str()), client)
    }

    fn http_of(&self, owner: Option<&str>, client: reqwest::Client) -> ProviderHttp {
        match (owner, self.devices.get()) {
            (Some(owner), Some(route)) => ProviderHttp::with_devices(client, route.clone(), owner),
            _ => ProviderHttp::new(client),
        }
    }

    pub fn rotation(&self) -> &Arc<AccountRotation> {
        &self.rotation
    }

    /// Marks `account` of `entry` used up until `until`, so no agent and no
    /// image takes it meanwhile, and tells the entry's pages now and at
    /// `until`, since it routes their requests elsewhere until then.
    pub fn mark_used_up(&self, entry: &ProviderId, account: &CredentialId, until: demi_shared_types::Timestamp) {
        self.rotation.mark_used_up(entry, account, until);
        self.quotas.used_up(entry, until);
    }

    /// Now, by the clock the assembly's choices read.
    pub fn now(&self) -> demi_shared_types::Timestamp {
        self.clock.now()
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    pub fn families(&self) -> &FamilyRegistry {
        &self.families
    }

    pub fn quotas(&self) -> &Arc<AccountQuotas> {
        &self.quotas
    }

    pub fn catalogs(&self) -> &ModelCatalogCache {
        &self.catalogs
    }

    pub fn vendors(&self) -> &VendorCatalog {
        &self.vendors
    }

    /// The vendors an entry can be added from, by name: models.dev's, and
    /// the media catalog's vendors of media alone whose shapes the `media`
    /// family builds, each with the endpoint the catalog names
    /// (`crates/backend/src/families.rs`); one whose shapes are not
    /// built yet, such as Stability AI's, is not offered.
    pub async fn offered_vendors(&self) -> Result<Vec<Vendor>, ModelsDevError> {
        let mut vendors = self.vendors.vendors().await?;
        for vendor in self.media_vendors() {
            if !vendors.iter().any(|known| known.id == vendor.id) {
                vendors.push(vendor);
            }
        }
        sort_by_name(&mut vendors);
        Ok(vendors)
    }

    /// The vendor `vendor_id`, when one is offered.
    pub async fn offered_vendor(&self, vendor_id: &str) -> Result<Option<Vendor>, ModelsDevError> {
        if let Some(vendor) = self.media_vendors().into_iter().find(|vendor| vendor.id == vendor_id) {
            return Ok(Some(vendor));
        }
        self.vendors.vendor(vendor_id).await
    }

    /// The media catalog's vendors that the `media` family makes something
    /// for.
    fn media_vendors(&self) -> Vec<Vendor> {
        let Some(family) = self.families.get(MEDIA_FAMILY) else {
            return Vec::new();
        };
        self.media
            .current()
            .vendors
            .iter()
            .filter(|vendor| {
                let id = Some(vendor.id.as_str());
                family.image_shape(id).is_some() || family.video_shape(id).is_some() || family.audio_shape(id).is_some()
            })
            .map(|vendor| Vendor {
                id: vendor.id.clone(),
                name: vendor.name.clone(),
                provider_type: MEDIA_FAMILY.to_owned(),
                wire_api: None,
                base_url: Some(vendor.endpoint.clone()),
                doc: vendor.doc.clone(),
            })
            .collect()
    }

    /// Whether `entry` makes text, as every family but `media` does: an
    /// entry of media alone has no text models and is never in the model
    /// menu (`crates/backend/src/families.rs`).
    pub fn makes_text(&self, entry: &ProviderEntry) -> bool {
        self.families.get(&entry.family).is_none_or(|family| family.makes_text())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<BuiltKey, Built>> {
        // A section only looks up or replaces one entry's provider.
        self.built.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The provider of `entry`, a fresh read of the entry, for its active
    /// account: the one built before while the entry is unchanged, otherwise
    /// a new one.
    pub async fn provider_for(
        &self,
        entry: &ProviderEntry,
    ) -> Result<Arc<dyn Provider>, AssemblyError> {
        self.provider_of(entry, entry.active()).await
    }

    /// The provider of `entry` for `account`: the one built before while
    /// the entry is unchanged, otherwise a new one.
    async fn provider_of(
        &self,
        entry: &ProviderEntry,
        account: Option<&CredentialId>,
    ) -> Result<Arc<dyn Provider>, AssemblyError> {
        let key = (entry.id.clone(), account.cloned());
        let reused = self
            .lock()
            .get(&key)
            .filter(|(built_from, _)| built_from == entry)
            .map(|(_, provider)| provider.clone());
        if let Some(provider) = reused {
            return Ok(provider);
        }
        let provider = self.build(entry, account).await?;
        // Two builds for one entry race only to insert the same provider.
        self.lock().insert(key, (entry.clone(), provider.clone()));
        Ok(provider)
    }

    /// The provider of `entry` for its account `account`, whichever is
    /// active; `None` when the entry holds no such account.
    pub async fn for_account(
        &self,
        entry: &ProviderEntry,
        account: &CredentialId,
    ) -> Result<Option<Arc<dyn Provider>>, AssemblyError> {
        if entry.active() == Some(account) {
            return self.provider_for(entry).await.map(Some);
        }
        let stored = self
            .vault
            .account(entry.id.clone(), account.clone())
            .await?;
        if stored.is_none() {
            return Ok(None);
        }
        self.provider_of(entry, Some(account)).await.map(Some)
    }

    /// The entry's accounts in the list's order with their snapshots, for
    /// a choice under its rotation, which passes over an account its vendor
    /// signed out (`crates/provider-claude-code/src/account.rs`); with `fresh`,
    /// a snapshot older than five
    /// minutes, or none, is read again first by a free probe, and one that
    /// fails leaves the snapshot as it was (`crates/backend-providers/src/llm/rotation.rs`).
    pub async fn candidates(
        &self,
        entry: &ProviderEntry,
        fresh: bool,
    ) -> Result<Vec<Candidate>, AssemblyError> {
        let mut records = self.vault.accounts(entry.id.clone()).await?;
        records.retain(|record| !record.signed_out);
        let now = self.clock.now().as_millisecond();
        let mut stale = Vec::new();
        for record in &records {
            let latest = self.quotas.latest(&entry.id, record);
            let old = latest
                .as_ref()
                .is_none_or(|snapshot| now - snapshot.observed_at.as_millisecond() > SNAPSHOT_FRESH_MS);
            if fresh && old {
                stale.push(self.provider_of(entry, Some(&record.id)).await?);
            }
        }
        // The probes run at the edge, whose client the providers send with,
        // together; the choice waits for all of them.
        let probes = stale.into_iter().map(|provider| {
            self.quotas.edge().spawn(async move {
                if let Some(quota) = provider.quota()
                    && let Err(error) = quota.probe().await
                {
                    tracing::debug!(%error, "an account's usage was not read before a choice; its snapshot stays");
                }
            })
        });
        for probe in futures_util::future::join_all(probes).await {
            // A probe that panicked read nothing; the snapshot stays.
            if let Err(error) = probe {
                tracing::warn!(%error, "an account's usage probe ended without an answer");
            }
        }
        Ok(records
            .iter()
            .map(|record| Candidate {
                id: record.id.clone(),
                label: record.label.clone(),
                snapshot: self.quotas.latest(&entry.id, record),
            })
            .collect())
    }

    /// Takes an account of `entry` under its rotation for an agent of
    /// `model`, reading stale snapshots first for Balanced; none for a
    /// Manual entry.
    pub async fn take_account(
        &self,
        entry: &ProviderEntry,
        model: &Model,
    ) -> Result<(Option<Choice>, Vec<Candidate>), AssemblyError> {
        let rotation = entry.rotation();
        let candidates = self
            .candidates(entry, rotation == Rotation::Balanced)
            .await?;
        let choice = self
            .rotation
            .take(&entry.id, rotation, &candidates, model, self.clock.now());
        Ok((choice, candidates))
    }

    /// The account operations a login into `entry` runs, whose sign-in runs
    /// on the Claude Code machine for a family whose login runs a process.
    pub async fn login_accounts(&self, entry: &ProviderEntry) -> Result<LoginAccounts, AssemblyError> {
        let family = self.family(&entry.family)?;
        let args = self.entry_args(entry, None).await?;
        match family.login_accounts(args) {
            Some(accounts) => Ok(LoginAccounts::Machine(accounts?)),
            None => Ok(LoginAccounts::Provider(self.provider_for(entry).await?)),
        }
    }

    /// The account operations of a login of `family` over `pool` with no
    /// account yet, whose entry does not exist until the login completes;
    /// its sign-in runs on the Claude Code machine for a family whose login
    /// runs a process.
    pub fn detached_login_accounts(
        &self,
        family: &str,
        id: &str,
        label: &str,
        pool: Arc<dyn CredentialPool>,
    ) -> Result<LoginAccounts, AssemblyError> {
        let registered = self.family(family)?;
        let args = || {
            let credential = FamilyCredential::Subscription(SubscriptionArgs {
                pool: pool.clone(),
                account: None,
            });
            // The entry does not exist yet: it has no owner whose devices
            // its requests could name.
            self.args(id.to_owned(), label.to_owned(), None, credential)
        };
        match registered.login_accounts(args()) {
            Some(accounts) => Ok(LoginAccounts::Machine(accounts?)),
            None => Ok(LoginAccounts::Provider(registered.provider(args())?)),
        }
    }

    /// Whether `entry`'s provider runs a process on a Host, which then is the
    /// Claude Code machine (`crates/provider-claude-code/src/placement.rs`).
    pub async fn runs_a_process(&self, entry: &ProviderEntry) -> Result<bool, AssemblyError> {
        Ok(self.provider_for(entry).await?.capabilities().process_host)
    }

    /// A session's runtime of `entry`'s provider for `account`, whose
    /// process `placement` starts, for a provider that runs one
    /// (`crates/provider-claude-code/src/placement.rs`).
    pub async fn process_runtime(
        &self,
        entry: &ProviderEntry,
        account: Option<&CredentialId>,
        env: RuntimeEnv,
        placement: Rc<dyn Placement>,
    ) -> Result<Box<dyn ProviderRuntime>, AssemblyError> {
        let family = self.family(&entry.family)?;
        let args = self.entry_args(entry, account).await?;
        let runtime = family
            .process_runtime(args, env, placement)
            .ok_or_else(|| AssemblyError::NoProcessRuntime(entry.family.clone()))?;
        Ok(runtime?)
    }

    /// Forgets the entry's provider and catalog after its configuration or
    /// active account changed.
    pub async fn invalidate(&self, id: &ProviderId) -> Result<(), StorageError> {
        self.lock().retain(|(built, _), _| built != id);
        self.catalogs.invalidate(id).await
    }

    /// Forgets everything held of a deleted entry.
    pub async fn forget(&self, id: &ProviderId) -> Result<(), StorageError> {
        self.quotas.forget_entry(id);
        self.rotation.forget_entry(id);
        self.invalidate(id).await
    }

    /// Stops the catalog refreshes and waits for the quota writes.
    pub async fn close(&self) {
        self.catalogs.close().await;
        self.quotas.close().await;
    }

    /// The entry as the web app sees it, with the endpoint its requests go
    /// to: its own, else, for a vendor entry, its family's default.
    pub fn provider_dto(&self, entry: &ProviderEntry) -> ProviderDto {
        let default = self
            .families
            .get(&entry.family)
            .and_then(|family| family.default_endpoint())
            // A family's default is a constant that parses.
            .map(|url| EndpointUrl::try_from(url.to_owned()).expect("a default endpoint parses"));
        entry.dto(default, self.media_shapes(entry), self.media.current().updated)
    }

    /// The shapes in which `entry` makes media: a compatible entry's, its
    /// user's choice; a vendor entry's, its family's for its vendor.
    pub fn media_shapes(&self, entry: &ProviderEntry) -> MediaShapes {
        let vendor = match &entry.credential {
            EntryCredential::ApiKey(config) if config.is_compatible() => {
                return MediaShapes { image: config.image_shape, video: config.video_shape, audio: config.audio_shape };
            }
            EntryCredential::ApiKey(config) => config.vendor_id.as_deref(),
            EntryCredential::Subscription { .. } => None,
        };
        let family = self.families.get(&entry.family);
        MediaShapes {
            image: family.and_then(|family| family.image_shape(vendor)),
            video: family.and_then(|family| family.video_shape(vendor)),
            audio: family.and_then(|family| family.audio_shape(vendor)),
        }
    }

    /// What makes `entry`'s media of `kind` with `provider`, its provider
    /// for the account a call takes: for a compatible entry, its shape's
    /// role, the one implementation every endpoint of that shape shares,
    /// offering the models of the kind its user added; a vendor entry's
    /// provider's own. None for an entry that makes none of the kind.
    pub fn maker(&self, entry: &ProviderEntry, provider: &Arc<dyn Provider>, kind: MediaKind) -> Option<MakerOf> {
        let EntryCredential::ApiKey(config) = &entry.credential else {
            return MakerOf::of(provider, kind);
        };
        if !config.is_compatible() {
            return MakerOf::of(provider, kind);
        }
        let base = config.base_url.as_ref()?.url().clone();
        let added = |of: ModelKind| -> Vec<MediaModelInfo> {
            config
                .models
                .iter()
                .flat_map(|models| &models.0)
                .filter(|model| model.kind == of)
                .map(|model| MediaModelInfo {
                    name: Some(model.display_name.as_str().to_owned()),
                    ..MediaModelInfo::id(model.id.as_str())
                })
                .collect()
        };
        match kind {
            MediaKind::Image => {
                if config.image_shape != Some(ImageShape::OpenaiImages) {
                    return None;
                }
                let make = self.families.compatible_images()?;
                Some(MakerOf::Images(make(CompatibleImagesArgs {
                    name: entry.label.clone(),
                    key: config.api_key.clone(),
                    base,
                    models: added(ModelKind::Image),
                    clock: self.clock.clone(),
                })))
            }
            MediaKind::Video => {
                let shape = config.video_shape?;
                let make = self.families.compatible_videos()?;
                Some(MakerOf::Videos(make(CompatibleVideosArgs {
                    name: entry.label.clone(),
                    shape,
                    key: config.api_key.clone(),
                    base,
                    models: added(ModelKind::Video),
                    clock: self.clock.clone(),
                })))
            }
            MediaKind::Audio => {
                if config.audio_shape != Some(AudioShape::OpenaiSpeech) {
                    return None;
                }
                let make = self.families.compatible_audio()?;
                Some(MakerOf::Audio(make(CompatibleAudioArgs {
                    name: entry.label.clone(),
                    key: config.api_key.clone(),
                    base,
                    models: added(ModelKind::Audio),
                    clock: self.clock.clone(),
                })))
            }
        }
    }

    /// The client the entries' providers send with, which lists an entry's
    /// image models for the settings page too.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn family(&self, name: &str) -> Result<&Arc<dyn ProviderFamily>, AssemblyError> {
        self.families
            .get(name)
            .ok_or_else(|| AssemblyError::UnknownFamily(name.to_owned()))
    }

    async fn build(
        &self,
        entry: &ProviderEntry,
        account: Option<&CredentialId>,
    ) -> Result<Arc<dyn Provider>, AssemblyError> {
        let family = self.family(&entry.family)?;
        let args = self.entry_args(entry, account).await?;
        Ok(family.provider(args)?)
    }

    /// What `entry`'s family builds from for `account`: the entry's settings,
    /// or its pool with the account and its quota store.
    async fn entry_args(
        &self,
        entry: &ProviderEntry,
        account: Option<&CredentialId>,
    ) -> Result<FamilyArgs, AssemblyError> {
        let credential = match &entry.credential {
            EntryCredential::ApiKey(config) => {
                // A provider is built from a vendor entry's key and a
                // compatible entry's endpoint; what the entry lacks is its
                // availability's to say.
                let lacks = config.is_compatible() && config.base_url.is_none()
                    || !config.is_compatible() && config.api_key.is_none();
                if lacks {
                    return Err(AssemblyError::Incomplete);
                }
                FamilyCredential::ApiKey(ApiKeyArgs {
                    api_key: config.api_key.clone(),
                    base_url: config
                        .base_url
                        .as_ref()
                        .map(|endpoint| endpoint.url().clone()),
                    wire_api: config.wire_api,
                    vendor: self.vendors.policy(config.vendor_id.as_deref()),
                    vendor_id: config.vendor_id.clone(),
                    image_shape: self.media_shapes(entry).image.filter(|_| !config.is_compatible()),
                    video_shape: self.media_shapes(entry).video.filter(|_| !config.is_compatible()),
                    audio_shape: self.media_shapes(entry).audio.filter(|_| !config.is_compatible()),
                })
            }
            EntryCredential::Subscription { .. } => {
                let binding = match account {
                    Some(account) => self
                        .vault
                        .account(entry.id.clone(), account.clone())
                        .await?
                        .map(|record| AccountBinding {
                            credential_id: record.id.as_str().to_owned(),
                            quota: self.quotas.store(&entry.id, &record),
                        }),
                    None => None,
                };
                FamilyCredential::Subscription(SubscriptionArgs {
                    pool: self.vault.pool(entry.id.clone()),
                    account: binding,
                })
            }
        };
        Ok(self.args(
            entry.id.as_str().to_owned(),
            entry.label.clone(),
            Some(entry.owner.as_str()),
            credential,
        ))
    }

    fn args(&self, entry_id: String, label: String, owner: Option<&str>, credential: FamilyCredential) -> FamilyArgs {
        FamilyArgs {
            entry_id,
            label,
            credential,
            http: self.http_of(owner, self.http.clone()),
            services_http: self.http.clone(),
            clock: self.clock.clone(),
            models_dev: self.vendors.models_dev().clone(),
            machine: self.machine.get().cloned(),
        }
    }
}

/// What makes an entry's media of one kind: its provider's own role, or
/// one of its own for a compatible entry. It holds what it makes with, so
/// a video job keeps it for as long as it runs.
#[derive(Clone)]
pub enum MakerOf {
    /// The provider's own role of the kind.
    Provider(Arc<dyn Provider>, MediaKind),
    Images(Arc<dyn ImageMaker>),
    Videos(Arc<dyn VideoMaker>),
    Audio(Arc<dyn AudioMaker>),
}

impl MakerOf {
    /// `provider`'s own role of `kind`, when it has one.
    fn of(provider: &Arc<dyn Provider>, kind: MediaKind) -> Option<Self> {
        let makes = match kind {
            MediaKind::Image => provider.images().is_some(),
            MediaKind::Video => provider.videos().is_some(),
            MediaKind::Audio => provider.audio().is_some(),
        };
        makes.then(|| Self::Provider(provider.clone(), kind))
    }

    /// What every kind's role says of itself.
    pub fn media(&self) -> &dyn MediaMaker {
        match self {
            Self::Provider(provider, MediaKind::Image) => provider.images().expect("a maker's provider makes images"),
            Self::Provider(provider, MediaKind::Video) => provider.videos().expect("a maker's provider makes videos"),
            Self::Provider(provider, MediaKind::Audio) => provider.audio().expect("a maker's provider makes audio"),
            Self::Images(maker) => maker.as_ref(),
            Self::Videos(maker) => maker.as_ref(),
            Self::Audio(maker) => maker.as_ref(),
        }
    }

    /// The image role, for a maker of images.
    pub fn images(&self) -> Option<&dyn ImageMaker> {
        match self {
            Self::Provider(provider, MediaKind::Image) => provider.images(),
            Self::Images(maker) => Some(maker.as_ref()),
            Self::Provider(_, MediaKind::Video | MediaKind::Audio) | Self::Videos(_) | Self::Audio(_) => None,
        }
    }

    /// The video role, for a maker of videos.
    pub fn videos(&self) -> Option<&dyn VideoMaker> {
        match self {
            Self::Provider(provider, MediaKind::Video) => provider.videos(),
            Self::Videos(maker) => Some(maker.as_ref()),
            Self::Provider(_, MediaKind::Image | MediaKind::Audio) | Self::Images(_) | Self::Audio(_) => None,
        }
    }

    /// The audio role, for a maker of audio.
    pub fn audio(&self) -> Option<&dyn AudioMaker> {
        match self {
            Self::Provider(provider, MediaKind::Audio) => provider.audio(),
            Self::Audio(maker) => Some(maker.as_ref()),
            Self::Provider(_, MediaKind::Image | MediaKind::Video) | Self::Images(_) | Self::Videos(_) => None,
        }
    }
}
