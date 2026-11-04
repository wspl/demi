//! The registry: the provider of each entry and account.
//!
//! An entry's family builds each provider from a fresh read of the entry. The
//! provider is reused only while the entry read is unchanged, so a lookup that
//! completes after an edit cannot keep later requests on a stale configuration.
//!
//! A provider stands for one entry and one credential, an API-key entry's key
//! or one subscription account.
//!
//! - The entry's active account is only the default for inference. A connection
//!   test, a usage probe and a refresh name the account they mean.
//! - A provider never changes accounts, so a request's quota observations land
//!   on the account that made it, and a request that outlives a switch cannot
//!   credit the wrong account.
//! - Selecting another account builds a new provider. A running request
//!   finishes with its own.

use demi_provider_common::media_catalog::MediaCatalog;
use demi_provider_common::Searches;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use demi_provider_common::credentials::{CredentialPool, SubscriptionAccounts};
use demi_provider_common::images::ImageMaker;
use demi_provider_common::media::{MediaMaker, MediaModelInfo};
use demi_provider_common::audio::AudioMaker;
use demi_provider_common::videos::VideoMaker;
use demi_provider_common::{DeviceRoute, ModelWires, Provider, ProviderHttp};
use demi_shared_types::{AudioShape, CacheTtl, Clock, MediaKind, MediaShapes, Model, Rotation};
use demi_web_api_protocol::ids::{CredentialId, ProviderId};
use demi_provider_common::models_dev::ModelsDevError;
use demi_web_api_protocol::providers::{ModelKind, ProviderDto, SETUP_MESSAGE, SetupPart, Vendor};
use demi_web_api_protocol::text::EndpointUrl;

use crate::catalog_cache::ModelCatalogCache;
use crate::rotation::{AccountRotation, Candidate, Choice};
use crate::families::{
    AccountBinding, ApiKeyArgs, CompatibleAudioArgs, FamilyArgs, FamilyCredential, FamilyError,
    FamilyRegistry, ProviderFamily, SubscriptionArgs,
};
use crate::vendors::{VendorCatalog, sort_by_name};

/// The family of the vendors that make media and no text (`crates/backend/src/families.rs`).
pub const MEDIA_FAMILY: &str = "media";
use crate::entry::{EntryCredential, ProviderEntry};
use crate::quotas::AccountQuotas;
use crate::store::{RegistryStore, StoreError};

/// Why an entry's provider could not be built.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(transparent)]
    Storage(#[from] StoreError),
    /// The entry names a family this backend does not register.
    #[error("the provider family {0} is not available")]
    UnknownFamily(String),
    #[error(transparent)]
    Family(#[from] FamilyError),
    /// An API-key entry without its key: nothing is sent until the user
    /// adds it.
    #[error("{SETUP_MESSAGE}")]
    Incomplete,
}

/// An entry's provider for one of its accounts, with the entry read and the
/// models' wire facts it was built from.
type Built = (ProviderEntry, ModelWires, Arc<dyn Provider>);

/// The provider of an entry and an account: the active one's, or another's
/// that an agent took under the entry's rotation.
type BuiltKey = (ProviderId, Option<CredentialId>);

/// How old a snapshot Smart compares may be before it is read again.
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
pub struct ProviderRegistry {
    store: Arc<dyn RegistryStore>,
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
    /// The media catalog, whose time each entry's media carry.
    media: MediaCatalog,
    /// The way to the users' devices, set once the backend's shards
    /// started, which hold their connections (`crates/backend-user-shard/src/shard/device_requests.rs`).
    devices: OnceLock<Arc<dyn DeviceRoute>>,
}

impl ProviderRegistry {
    pub fn new(
        store: Arc<dyn RegistryStore>,
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
            store,
            families,
            quotas,
            catalogs,
            vendors,
            http,
            clock,
            built: Mutex::default(),
            rotation: AccountRotation::new(),
            devices: OnceLock::new(),
        }
    }

    /// Sets the way to the users' devices, once, as the backend's shards start.
    ///
    /// A second setting is refused and the first kept.
    pub fn set_devices(&self, route: Arc<dyn DeviceRoute>) {
        if self.devices.set(route).is_err() {
            tracing::error!(event = "provider_registry.set_twice", what = "devices", "the way to the devices was set twice; the first is kept");
        }
    }

    /// The client `entry`'s requests go through on `client`, the shard's or
    /// the shared services': out from the owner's device the entry connects
    /// through (`crates/backend-user-shard/src/shard/device_requests.rs`), or
    /// to the network.
    pub fn http_for(&self, entry: &ProviderEntry, client: reqwest::Client) -> ProviderHttp {
        self.http_of(Some(entry), client)
    }

    fn http_of(&self, entry: Option<&ProviderEntry>, client: reqwest::Client) -> ProviderHttp {
        let through = entry.and_then(|entry| Some((entry, entry.through_device.as_ref()?)));
        let http = match (through, self.devices.get()) {
            (Some((entry, device)), Some(route)) => {
                ProviderHttp::through_device(client, route.clone(), entry.owner.as_str(), device.as_str())
            }
            _ => ProviderHttp::new(client),
        };
        // An entry's headers go to its endpoint: its own, else a vendor
        // entry's family's default. A compatible entry that has headers but
        // no endpoint yet has no request to.
        let own = entry.and_then(|entry| match &entry.credential {
            EntryCredential::ApiKey(config) => {
                let endpoint = match &config.base_url {
                    Some(endpoint) => endpoint.url().clone(),
                    None if !config.is_compatible() => {
                        reqwest::Url::parse(self.families.get(&entry.family)?.default_endpoint()?).ok()?
                    }
                    None => return None,
                };
                Some((entry.headers(), endpoint))
            }
            EntryCredential::Subscription { .. } => None,
        });
        match own {
            Some((headers, endpoint)) => http.with_headers(headers, &endpoint),
            None => http,
        }
    }

    pub fn rotation(&self) -> &Arc<AccountRotation> {
        &self.rotation
    }

    /// Marks `account` of `entry` used up until `until`, as its vendor just
    /// refused it, so requests take its other accounts first meanwhile, and
    /// tells the entry's pages now and at `until`.
    ///
    /// `source` says where `until` comes from: `vendor_refusal` when the
    /// failure named it, `snapshot` when the account's quota did, `default`
    /// when neither did.
    pub fn mark_used_up(&self, entry: &ProviderId, account: &CredentialId, until: demi_shared_types::Timestamp, source: &'static str) {
        tracing::info!(
            event = "account.used_up",
            provider = %entry,
            account = %account,
            until_ms = until.as_millisecond(),
            source,
            "an account was marked used up"
        );
        self.rotation.mark_used_up(entry, account, until, self.clock.now());
        self.quotas.used_up(entry, until);
    }

    /// Now, by the clock the registry's choices read.
    pub fn now(&self) -> demi_shared_types::Timestamp {
        self.clock.now()
    }

    /// What the program stores for the registry.
    pub fn store(&self) -> &dyn RegistryStore {
        &*self.store
    }

    /// The store, for a task that outlives the call, such as a catalog's
    /// read.
    pub(crate) fn shared_store(&self) -> Arc<dyn RegistryStore> {
        self.store.clone()
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

    /// The vendors an entry can be added from, by name:
    ///
    /// - models.dev's vendors.
    /// - The media catalog's vendors of media alone whose shapes the `media`
    ///   family builds, each with the endpoint the catalog names
    ///   (`crates/backend/src/families.rs`).
    ///
    /// A media vendor whose shapes are not built yet, such as Stability AI's,
    /// is not offered.
    pub async fn offered_vendors(&self) -> Result<Vec<Vendor>, ModelsDevError> {
        let mut vendors: Vec<Vendor> = self.vendors.vendors().await?.into_iter().map(|vendor| self.with_endpoint(vendor)).collect();
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
        Ok(self.vendors.vendor(vendor_id).await?.map(|vendor| self.with_endpoint(vendor)))
    }

    /// `vendor` with the endpoint its entry starts with: the one models.dev
    /// names, else its family's default, as a family's own vendor's is.
    fn with_endpoint(&self, vendor: Vendor) -> Vendor {
        let default = || self.families.get(&vendor.provider_type).and_then(|family| family.default_endpoint()).map(str::to_owned);
        let base_url = vendor.base_url.clone().or_else(default);
        Vendor { base_url, ..vendor }
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

    /// Whether `entry`'s catalog is its provider's directory
    /// ([`ProviderFamily::reads_directory`]).
    pub fn reads_directory(&self, entry: &ProviderEntry) -> bool {
        self.families.get(&entry.family).is_some_and(|family| family.reads_directory())
    }

    /// What `entry` lacks to infer ([`ProviderEntry::missing`]).
    pub fn missing(&self, entry: &ProviderEntry) -> Vec<SetupPart> {
        entry.missing(self.reads_directory(entry))
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
    ) -> Result<Arc<dyn Provider>, RegistryError> {
        self.provider_of(entry, entry.active()).await
    }

    /// The provider of `entry` for `account`: the one built before while
    /// the entry and its models' wire facts are unchanged, otherwise a new
    /// one. The facts count because a vendor's come from the models.dev copy,
    /// which a provider built without one would otherwise miss until the
    /// entry changes.
    async fn provider_of(
        &self,
        entry: &ProviderEntry,
        account: Option<&CredentialId>,
    ) -> Result<Arc<dyn Provider>, RegistryError> {
        let key = (entry.id.clone(), account.cloned());
        let models = self.model_wires(entry).await;
        let reused = self
            .lock()
            .get(&key)
            .filter(|(built_from, built_models, _)| built_from == entry && *built_models == models)
            .map(|(_, _, provider)| provider.clone());
        if let Some(provider) = reused {
            return Ok(provider);
        }
        let family = self.family(&entry.family)?;
        let args = self.entry_args(entry, account, models.clone()).await?;
        let provider = family.provider(args)?;
        // Two builds for one entry race only to insert the same provider.
        self.lock().insert(key, (entry.clone(), models, provider.clone()));
        Ok(provider)
    }

    /// How each model of `entry` wants thinking sent and sent back: a
    /// compatible entry's as its user set them; a vendor's from models.dev,
    /// whether the entry follows the vendor's list or keeps its own, so a
    /// model models.dev does not list takes the defaults.
    async fn model_wires(&self, entry: &ProviderEntry) -> ModelWires {
        let EntryCredential::ApiKey(config) = &entry.credential else {
            return ModelWires::default();
        };
        match config.vendor_id.as_deref() {
            Some(vendor_id) => self.vendors.wires(vendor_id).await,
            None => ModelWires::new(
                config
                    .models
                    .iter()
                    .flat_map(|models| &models.0)
                    .filter(|model| model.kind == ModelKind::Text)
                    .map(|model| (model.id.as_str().to_owned(), model.wire()))
                    .collect(),
            ),
        }
    }

    /// The provider of `entry` for its account `account`, whichever is
    /// active; `None` when the entry holds no such account.
    pub async fn for_account(
        &self,
        entry: &ProviderEntry,
        account: &CredentialId,
    ) -> Result<Option<Arc<dyn Provider>>, RegistryError> {
        if entry.active() == Some(account) {
            return self.provider_for(entry).await.map(Some);
        }
        let stored = self
            .store
            .account(entry.id.clone(), account.clone())
            .await?;
        if stored.is_none() {
            return Ok(None);
        }
        self.provider_of(entry, Some(account)).await.map(Some)
    }

    /// The entry's accounts in the list's order with their snapshots, for a
    /// choice under its rotation, which passes over an account its vendor
    /// signed out (`crates/provider-claude-code/src/account.rs`).
    ///
    /// With `fresh`, a snapshot older than five minutes, or none, is read again
    /// first by a free probe, and one that fails leaves the snapshot as it was
    /// (`crates/provider-registry/src/rotation.rs`).
    pub async fn candidates(
        &self,
        entry: &ProviderEntry,
        fresh: bool,
    ) -> Result<Vec<Candidate>, RegistryError> {
        let mut records = self.store.accounts(entry.id.clone()).await?;
        records.retain(|record| !record.signed_out);
        let now = self.clock.now().as_millisecond();
        let mut stale = Vec::new();
        for record in &records {
            let latest = self.quotas.latest(&entry.id, record);
            let old = latest
                .as_ref()
                .is_none_or(|snapshot| now - snapshot.observed_at.as_millisecond() > SNAPSHOT_FRESH_MS);
            if fresh && old {
                stale.push((record.id.clone(), self.provider_of(entry, Some(&record.id)).await?));
            }
        }
        // The probes run at the edge, whose client the providers send with,
        // together; the choice waits for all of them.
        let probes = stale.into_iter().map(|(account, provider)| {
            let provider_id = entry.id.clone();
            let probed = account.clone();
            let probe = self.quotas.edge().spawn(async move {
                if let Some(quota) = provider.quota()
                    && let Err(error) = quota.probe().await
                {
                    tracing::warn!(event = "account_usage.read_failed", provider = %provider_id, account = %probed, kept = "snapshot", %error, "an account's usage was not read before a choice; its snapshot stays");
                }
            });
            async move { (account, probe.await) }
        });
        for (account, probe) in futures_util::future::join_all(probes).await {
            // A probe that panicked read nothing; the snapshot stays.
            if let Err(error) = probe {
                tracing::warn!(event = "account_usage.probe_failed", provider = %entry.id, account = %account, %error, "an account's usage probe ended without an answer");
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

    /// Takes an account of `entry` under its rotation for a request of
    /// `model`, other than those in `tried`, which refused it already,
    /// reading stale snapshots first for Smart; none for a Manual entry.
    pub async fn take_account(
        &self,
        entry: &ProviderEntry,
        model: &Model,
        tried: &[CredentialId],
    ) -> Result<(Option<Choice>, Vec<Candidate>), RegistryError> {
        let rotation = entry.rotation();
        let candidates = self
            .candidates(entry, rotation == Rotation::Smart)
            .await?;
        let choice = self
            .rotation
            .take(&entry.id, rotation, &candidates, model, tried, self.clock.now());
        Ok((choice, candidates))
    }

    /// The account operations a login into `entry` runs, whose sign-in runs
    /// a process of its family's own for a family whose login runs one.
    pub async fn login_accounts(&self, entry: &ProviderEntry) -> Result<LoginAccounts, RegistryError> {
        let family = self.family(&entry.family)?;
        let args = self.entry_args(entry, None, self.model_wires(entry).await).await?;
        match family.login_accounts(args) {
            Some(accounts) => Ok(LoginAccounts::Machine(accounts?)),
            None => Ok(LoginAccounts::Provider(self.provider_for(entry).await?)),
        }
    }

    /// The account operations of a login of `family` over `pool` with no
    /// account yet, whose entry does not exist until the login completes;
    /// its sign-in runs a process of its family's own for a family whose
    /// login runs one.
    pub fn detached_login_accounts(
        &self,
        family: &str,
        id: &str,
        label: &str,
        pool: Arc<dyn CredentialPool>,
    ) -> Result<LoginAccounts, RegistryError> {
        let registered = self.family(family)?;
        let args = || {
            let credential = FamilyCredential::Subscription(SubscriptionArgs {
                pool: pool.clone(),
                account: None,
            });
            // The entry does not exist yet: it connects through no device.
            self.args(id.to_owned(), label.to_owned(), None, credential)
        };
        match registered.login_accounts(args()) {
            Some(accounts) => Ok(LoginAccounts::Machine(accounts?)),
            None => Ok(LoginAccounts::Provider(registered.provider(args())?)),
        }
    }

    /// Whether `entry`'s provider runs its CLI as a process on a machine.
    pub async fn runs_a_process(&self, entry: &ProviderEntry) -> Result<bool, RegistryError> {
        Ok(self.provider_for(entry).await?.capabilities().cli_package.is_some())
    }

    /// A provider of `entry` for `account` of its own, built without the
    /// cache, as a session's runtime of a provider that runs a process has
    /// one.
    pub async fn own_provider(
        &self,
        entry: &ProviderEntry,
        account: Option<&CredentialId>,
    ) -> Result<Arc<dyn Provider>, RegistryError> {
        let family = self.family(&entry.family)?;
        let args = self.entry_args(entry, account, self.model_wires(entry).await).await?;
        Ok(family.provider(args)?)
    }

    /// Forgets the entry's provider and catalog after its configuration or
    /// active account changed.
    pub async fn invalidate(&self, id: &ProviderId) -> Result<(), StoreError> {
        self.lock().retain(|(built, _), _| built != id);
        self.catalogs.invalidate(id).await
    }

    /// Forgets everything held of a deleted entry.
    pub async fn forget(&self, id: &ProviderId) -> Result<(), StoreError> {
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
        let mut dto = entry.dto(
            default,
            self.media_shapes(entry),
            self.offered_searches(entry),
            self.media.current().updated,
            self.cache_ttl_presets(entry),
            self.vendor_headers(entry),
        );
        // An entry whose models its provider's directory lists, as a
        // borrowed one lists its lender's, is no compatible entry: neither
        // its endpoint nor its models are the user's to choose.
        dto.compatible &= !self.reads_directory(entry);
        dto
    }

    /// The headers a vendor `entry` may set, as its family declares them;
    /// none for a compatible entry, which sets any, and a subscription.
    pub fn vendor_headers(&self, entry: &ProviderEntry) -> &'static [&'static str] {
        match &entry.credential {
            EntryCredential::ApiKey(config) if !config.is_compatible() => self
                .families
                .get(&entry.family)
                .map_or(&[], |family| family.vendor_headers(config.vendor_id.as_deref())),
            _ => &[],
        }
    }

    /// The prompt cache lifetimes `entry`'s protocol knows, as its family and
    /// wire say; none for an entry whose requests name none.
    pub fn cache_ttl_presets(&self, entry: &ProviderEntry) -> Option<Vec<CacheTtl>> {
        let wire = match &entry.credential {
            EntryCredential::ApiKey(config) => config.wire_api,
            EntryCredential::Subscription { .. } => None,
        };
        self.families.get(&entry.family)?.cache_ttl_presets(wire)
    }

    /// The searches `entry`'s vendor has, which its switches turn on.
    pub fn offered_searches(&self, entry: &ProviderEntry) -> Searches {
        self.families.get(&entry.family).map(|family| family.searches()).unwrap_or_default()
    }

    /// The shapes in which `entry` makes media: a compatible entry's, the
    /// audio shape its user chose; a vendor entry's, its family's for its
    /// vendor; none for a borrowed one, whose lender's are its own
    /// ([`Self::makes`]).
    pub fn media_shapes(&self, entry: &ProviderEntry) -> MediaShapes {
        if self.reads_directory(entry) {
            return MediaShapes::default();
        }
        let vendor = match &entry.credential {
            EntryCredential::ApiKey(config) if config.is_compatible() => {
                return MediaShapes { image: None, video: None, audio: config.audio_shape };
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

    /// Whether `entry` may make media of `kind`: its shapes say so, or its
    /// provider's directory lists what it makes, as a borrowed entry's lists
    /// its lender's, whose maker lists no models of a kind it does not make.
    pub fn makes(&self, entry: &ProviderEntry, kind: MediaKind) -> bool {
        self.reads_directory(entry) || self.media_shapes(entry).makes(kind)
    }

    /// What makes `entry`'s media of `kind` with `provider`, its provider for
    /// the account a call takes.
    ///
    /// - For a compatible entry, it is its audio shape's role: the one
    ///   implementation every endpoint of that shape shares, offering the
    ///   audio models its user added. A compatible entry makes no images and
    ///   no videos: a gateway that serves a vendor's is that vendor's entry
    ///   with the gateway's endpoint.
    /// - For a vendor entry, and an entry whose provider's directory lists
    ///   its models, as a borrowed one's lists its lender's, it is its
    ///   provider's own.
    ///
    /// The result is none for an entry that makes none of the kind.
    pub fn maker(&self, entry: &ProviderEntry, provider: &Arc<dyn Provider>, kind: MediaKind) -> Option<MakerOf> {
        let EntryCredential::ApiKey(config) = &entry.credential else {
            return MakerOf::of(provider, kind);
        };
        if !config.is_compatible() || self.reads_directory(entry) {
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
            MediaKind::Image | MediaKind::Video => None,
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

    pub fn family(&self, name: &str) -> Result<&Arc<dyn ProviderFamily>, RegistryError> {
        self.families
            .get(name)
            .ok_or_else(|| RegistryError::UnknownFamily(name.to_owned()))
    }

    /// What `entry`'s family builds from for `account`: the entry's settings
    /// with its models' wire facts `models`, or its pool with the account and
    /// its quota store.
    async fn entry_args(
        &self,
        entry: &ProviderEntry,
        account: Option<&CredentialId>,
        models: ModelWires,
    ) -> Result<FamilyArgs, RegistryError> {
        let credential = match &entry.credential {
            EntryCredential::ApiKey(config) => {
                // A provider is built from a vendor entry's key and a
                // compatible entry's endpoint; what the entry lacks is its
                // availability's to say.
                let lacks = config.is_compatible() && config.base_url.is_none()
                    || !config.is_compatible() && config.api_key.is_none();
                if lacks {
                    return Err(RegistryError::Incomplete);
                }
                FamilyCredential::ApiKey(ApiKeyArgs {
                    api_key: config.api_key.clone(),
                    base_url: config
                        .base_url
                        .as_ref()
                        .map(|endpoint| endpoint.url().clone()),
                    wire_api: config.wire_api,
                    models,
                    vendor_id: config.vendor_id.clone(),
                    image_shape: self.media_shapes(entry).image.filter(|_| !config.is_compatible()),
                    video_shape: self.media_shapes(entry).video.filter(|_| !config.is_compatible()),
                    audio_shape: self.media_shapes(entry).audio.filter(|_| !config.is_compatible()),
                })
            }
            EntryCredential::Subscription { .. } => {
                let binding = match account {
                    Some(account) => self
                        .store
                        .account(entry.id.clone(), account.clone())
                        .await?
                        .map(|record| AccountBinding {
                            credential_id: record.id.as_str().to_owned(),
                            quota: self.quotas.store(&entry.id, &record),
                        }),
                    None => None,
                };
                FamilyCredential::Subscription(SubscriptionArgs {
                    pool: self.store.pool(entry.id.clone()),
                    account: binding,
                })
            }
        };
        let mut args = self.args(
            entry.id.as_str().to_owned(),
            entry.label.clone(),
            Some(entry),
            credential,
        );
        args.cache_ttl = entry.cache_ttl.clone();
        args.searches = entry.searches(self.offered_searches(entry));
        Ok(args)
    }

    fn args(&self, entry_id: String, label: String, entry: Option<&ProviderEntry>, credential: FamilyCredential) -> FamilyArgs {
        FamilyArgs {
            entry_id,
            label,
            credential,
            cache_ttl: None,
            searches: Searches::default(),
            http: self.http_of(entry, self.http.clone()),
            services_http: self.http.clone(),
            clock: self.clock.clone(),
            models_dev: self.vendors.models_dev().clone(),
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
