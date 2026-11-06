//! The changes of the provider entries in a user's scope, and the model search
//! that fills a configured model
//! (`crates/provider-registry/src/vendors.rs`).
//!
//! There is one path, with its checks and its words, for the settings page and
//! for `demi providers` (`crates/core/src/provider_commands.rs`).
//!
//! An entry may be added with nothing but its kind; it is incomplete until it has
//! what it needs.
//!
//! # Vendor and compatible entries
//!
//! A vendor entry and a compatible one differ in who decides the models:
//!
//! - A vendor's own API serves the vendor's catalog, so its entry lists them at
//!   once.
//! - A compatible API serves whatever models it chooses under whatever names, so
//!   its entry starts empty and never borrows the protocol's vendor's catalog.
//!
//! # Rules
//!
//! - An entry stays the kind it was added as.
//! - An entry that cannot infer yet is incomplete and shows what it lacks
//!   plainly, never as a failure, since nothing failed.
//! - Nothing is checked against the vendor while the user types.
//! - A model added with a taken id takes the first free `-2`, `-3` suffix.

use demi_core_store::StorageError;
use demi_core_store::providers::EntrySwitches;
use demi_provider_registry::families::{ProviderFamily, family_name};
use demi_provider_registry::entry::{ApiKeyConfig, EntryCredential, ProviderEntry};
use demi_core_providers::vault::entries::Vault;
use demi_core_providers::vault::operations::{Held, OperationGuard};
use demi_provider_common::credentials::MemoryCredentialPool;
use demi_provider_common::{ProviderHttp, Secret, SecretError};
use demi_provider_relay::borrow::{Borrowed, FAMILY as BORROWED, server_name};
use demi_provider_relay::key::{NotAKey, ShareKey};
use demi_shared_types::{AudioShape, CacheTtl, MediaKind, MediaShapes, WireApi};
use demi_web_api_protocol::ids::{DeviceId, ProviderId, UserId};
use demi_web_api_protocol::providers::{
    ConfiguredModel, ConfiguredModels, CreateProvider, CredentialKind, ModelKind, ModelMatch, ProviderHeader, ProviderPatch,
    Rotation, Vendor,
};
use demi_web_api_protocol::text::{EndpointUrl, Trimmed};
use std::sync::Arc;

use crate::services::CoreServices;
use crate::starter::Starter;

/// Why a change of an entry, or a search, was refused, in the words both
/// the settings page and `demi providers` show.
#[derive(Debug, thiserror::Error)]
pub enum ProviderRefusal {
    /// The user only infers with the scope's entries.
    #[error("Providers are configured by the instance owner")]
    NotConfiguring,
    #[error("No such provider")]
    NotFound,
    /// A change of a borrowed entry besides its name and share key; it
    /// names the entry.
    #[error("{0} is shared by another server; only its name and share key can be changed.")]
    BorrowedFixed(String),
    /// A share key for an entry no other server shares; it names the entry.
    #[error("{0} is not shared by another server; it takes no share key.")]
    NotBorrowed(String),
    /// A share key that is not one, or that its lender refused.
    #[error("{0}")]
    ShareKey(String),
    /// Sharing an entry another server shares; it names the entry.
    #[error("{0} is shared by another server; it can’t be shared again.")]
    Borrowed(String),
    /// Sharing while the server cannot say where it is reached.
    #[error("{0}")]
    Unlendable(String),
    #[error("Another change of this provider is still running")]
    Busy,
    #[error("Unknown provider type \"{0}\"")]
    UnknownFamily(String),
    /// A subscription family named as an API-key entry's.
    #[error("Provider type \"{0}\" is configured by its login")]
    LoginFamily(String),
    /// An API-key family named as a subscription's.
    #[error("Provider type \"{0}\" takes an API key, not a subscription")]
    KeyFamily(String),
    #[error("wireApi: the {family} family does not speak {wire}")]
    Wire { family: String, wire: WireApi },
    #[error("Unknown vendor \"{0}\"")]
    UnknownVendor(String),
    /// The models.dev document could not be read.
    #[error("{0}")]
    CatalogUnavailable(String),
    #[error("apiKey: {0}")]
    Key(#[from] SecretError),
    /// A second subscription entry of a family; it names the family.
    #[error("This scope already has a {0} subscription")]
    Exists(String),
    /// A key or an endpoint for a subscription entry.
    #[error("A subscription entry takes no key or endpoint")]
    SubscriptionOnly,
    /// A model list for a subscription entry, whose models come from its
    /// provider; it names the entry.
    #[error("{0} lists its own models; its list cannot be edited.")]
    OwnModels(String),
    /// A protocol for a vendor entry, which speaks its vendor's; it names
    /// the entry.
    #[error("{0} speaks its vendor’s protocol; it can’t be changed.")]
    VendorProtocol(String),
    /// A header a vendor entry's family does not declare; it names the
    /// entry and the header.
    #[error("{entry} takes no {header} header; a vendor’s entry sends only the headers its vendor documents.")]
    VendorHeader { entry: String, header: String },
    /// A list of headers with a name or a value no request can carry, as
    /// [`ProviderHeader::problem`] says it.
    #[error("{0}")]
    Headers(String),
    /// A rotation other than manual for an entry with fewer than two
    /// accounts.
    #[error("Rotation needs at least two accounts")]
    RotationNeedsAccounts,
    /// A switch of a kind for an entry that makes none of it; it names the
    /// entry and the kind.
    #[error("{0} doesn’t make {1}.")]
    MakesNo(String, &'static str),
    /// A switch of a search the entry's vendor does not have; it names the
    /// entry and what the search would search.
    #[error("{0} can’t search {1}.")]
    SearchesNo(String, &'static str),
    /// A shape for a vendor entry, whose shapes are its vendor's; it names
    /// the entry.
    #[error("{0} makes media as its vendor does; its shapes can’t be changed.")]
    ShapeFixed(String),
    /// A model of a kind the entry makes none of.
    #[error("models: {model} is {} {kind} model, but {entry} makes no {}; choose {} {kind} shape first", article(*kind), plural(*kind), article(*kind))]
    UnmadeKind { entry: String, model: String, kind: ModelKind },
    /// A device to connect through that is none of the entry owner's.
    #[error("throughDevice: no such device of the provider's owner")]
    UnknownDevice,
    /// A device for an entry whose family's requests the backend does not
    /// make with the entry's client; it names the entry.
    #[error("throughDevice: {0} makes its requests another way; it can’t connect through a device")]
    ConnectsDirectly(String),
    /// A prompt cache lifetime the entry's provider does not take.
    #[error("cacheTtl: {entry} can’t ask for a prompt cache lifetime of {ttl}")]
    CacheTtl { entry: String, ttl: String },
    /// A list of entries for an order that does not name each of the
    /// scope's once.
    #[error("providerIds: name every provider once")]
    Order,
    /// A model list that names one id twice; it names the id.
    #[error("models: the model ID {0:?} appears twice")]
    DuplicateModel(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Store(#[from] demi_provider_registry::store::StoreError),
}

/// The article before a kind of model: a text model, an audio model.
fn article(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Audio => "an",
        ModelKind::Text => "a",
    }
}

/// What an entry makes of a kind of model, as a sentence names it: text,
/// audio.
fn plural(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Text => "text",
        ModelKind::Audio => "audio",
    }
}

impl CoreServices {
    /// Refuses a user who infers with another's entries of `kind`, and so
    /// adds none.
    pub async fn may_add(&self, user: &UserId, kind: CredentialKind) -> Result<(), ProviderRefusal> {
        if self.vault.owners(user).await?.manages(kind) {
            Ok(())
        } else {
            Err(ProviderRefusal::NotConfiguring)
        }
    }

    /// The entry `id` when `user` infers with it and manages it; one they
    /// only infer with is refused, and any other reads as missing.
    pub async fn managed_entry(
        &self,
        user: &UserId,
        id: &str,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        let entry = self.scoped_entry(user, id).await?;
        if Vault::manages(user, &entry) {
            Ok(entry)
        } else {
            Err(ProviderRefusal::NotConfiguring)
        }
    }

    /// The entry `id` when it is one of `user`'s scope; any other reads as
    /// missing.
    pub async fn scoped_entry(
        &self,
        user: &UserId,
        id: &str,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        let id = ProviderId::try_from(id).map_err(|_| ProviderRefusal::NotFound)?;
        self.vault
            .visible(user, &id)
            .await?
            .ok_or(ProviderRefusal::NotFound)
    }

    /// Holds `entry` for one change.
    pub fn hold_entry(&self, entry: &ProviderEntry) -> Result<OperationGuard, ProviderRefusal> {
        self.operations
            .reserve(&entry.id)
            .ok_or(ProviderRefusal::Busy)
    }

    /// Creates an entry: an API-key entry, a vendor entry from the vendor list,
    /// or from a family alone for its own vendor, or a compatible entry,
    /// Anthropic Messages unless the body names another protocol; or a
    /// subscription entry with no account.
    ///
    /// Its label is the vendor's or the family's name, or *Compatible API*, unless
    /// the body gives one. What the body leaves out leaves it incomplete.
    pub async fn create_provider(
        &self,
        user: &UserId,
        body: CreateProvider,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        let kind = match &body {
            CreateProvider::Subscription { .. } => CredentialKind::Subscription,
            _ => CredentialKind::ApiKey,
        };
        self.may_add(user, kind).await?;
        let (family, label, config) = match body {
            CreateProvider::Subscription {
                provider_type,
                through_device,
            } => return self.create_subscription(user, provider_type, through_device).await,
            CreateProvider::Vendor {
                vendor_id,
                label,
                api_key,
                models,
            } => {
                let vendor = self
                    .provider_registry
                    .offered_vendor(&vendor_id)
                    .await
                    .map_err(|error| ProviderRefusal::CatalogUnavailable(error.to_string()))?
                    .ok_or(ProviderRefusal::UnknownVendor(vendor_id))?;
                self.api_key_family(&vendor.provider_type, vendor.wire_api)?;
                let base_url = vendor_endpoint(&vendor)?;
                let label = label.ok_or(vendor.name);
                let config = ApiKeyConfig {
                    api_key: api_key.map(key).transpose()?,
                    base_url,
                    wire_api: vendor.wire_api,
                    vendor_id: Some(vendor.id),
                    models,
                    audio_shape: None,
                    headers: Vec::new(),
                };
                (vendor.provider_type, label, config)
            }
            CreateProvider::Family {
                provider_type,
                wire_api,
                label,
                api_key,
                models,
            } => {
                self.api_key_family(&provider_type, wire_api)?;
                let label = label.ok_or_else(|| family_name(&provider_type).to_owned());
                // An API-key family is named after its own vendor's models.dev id.
                let config = ApiKeyConfig {
                    api_key: api_key.map(key).transpose()?,
                    base_url: None,
                    wire_api,
                    vendor_id: Some(provider_type.clone()),
                    models,
                    audio_shape: None,
                    headers: Vec::new(),
                };
                (provider_type, label, config)
            }
            CreateProvider::Compatible {
                provider_type,
                wire_api,
                label,
                api_key,
                base_url,
                models,
            } => {
                let provider_type = provider_type.unwrap_or_else(|| COMPATIBLE_FAMILY.to_owned());
                self.api_key_family(&provider_type, wire_api)?;
                let label = label.ok_or_else(|| COMPATIBLE_LABEL.to_owned());
                let config = ApiKeyConfig {
                    api_key: api_key.map(key).transpose()?,
                    base_url,
                    wire_api,
                    vendor_id: None,
                    models,
                    audio_shape: None,
                    headers: Vec::new(),
                };
                (provider_type, label, config)
            }
            CreateProvider::Borrowed { label } => {
                let label = label.ok_or_else(|| BORROWED_LABEL.to_owned());
                let config = ApiKeyConfig {
                    api_key: None,
                    base_url: None,
                    wire_api: None,
                    vendor_id: None,
                    models: None,
                    audio_shape: None,
                    headers: Vec::new(),
                };
                (BORROWED.to_owned(), label, config)
            }
        };
        distinct_models(config.models.as_ref())?;
        let cache_ttl = self
            .provider_registry
            .families()
            .get(&family)
            .and_then(|family| family.default_cache_ttl());
        let owner = user.clone();
        let label = match label {
            Ok(given) => given.into_string(),
            Err(default) => {
                let entries = self.vault.entries(owner.clone()).await?;
                free_label(default, |taken| entries.iter().any(|entry| entry.label == taken))
            }
        };
        Ok(self
            .vault
            .create_api_key(owner, family, label, config, cache_ttl)
            .await?)
    }

    /// Creates `owner`'s subscription entry of `family` with no account,
    /// connecting through `through_device`; the user's sign-ins then add its
    /// accounts.
    async fn create_subscription(
        &self,
        owner: &UserId,
        family: String,
        through_device: Option<DeviceId>,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        match self.provider_registry.families().get(&family) {
            None => return Err(ProviderRefusal::UnknownFamily(family)),
            Some(registered) if registered.credential() != CredentialKind::Subscription => {
                return Err(ProviderRefusal::KeyFamily(family));
            }
            Some(_) => {}
        }
        // A login making the family's entry holds it, and makes it itself.
        let _held = self
            .operations
            .hold(Held::NewEntry { owner: owner.clone(), family: family.clone() }, None)
            .map_err(|_| ProviderRefusal::Busy)?;
        let label = family_name(&family).to_owned();
        let through_device = self.through_device(owner, &family, &label, through_device).await?;
        self.vault
            .create_subscription(owner.clone(), family.clone(), label, through_device, &MemoryCredentialPool::new())
            .await?
            .ok_or(ProviderRefusal::Exists(family))
    }

    /// Edits the entry `id`: the label of any; the key, model list and
    /// endpoint of an API-key entry, a vendor entry's returning to its
    /// vendor's when removed; and a compatible entry's protocol, whose change
    /// of family keeps its key, endpoint and models. A configuration edit
    /// starts the entry's provider and catalog afresh.
    pub async fn update_provider(
        &self,
        user: &UserId,
        id: &str,
        patch: ProviderPatch,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        let entry = self.managed_entry(user, id).await?;
        let _held = self.hold_entry(&entry)?;
        if entry.family == BORROWED {
            return self.update_borrowed(user, entry, patch).await;
        }
        if patch.share_key.is_some() {
            return Err(ProviderRefusal::NotBorrowed(entry.label.clone()));
        }
        let protocol = patch.provider_type.is_some() || patch.wire_api.is_some();
        let audio_shape = patch.media.as_ref().and_then(|media| media.audio.as_ref()).and_then(|audio| audio.shape);
        let shaped = audio_shape.is_some();
        let lists = patch.models.is_some();
        let reconfigures = patch.api_key.is_some()
            || patch.base_url.is_some()
            || patch.headers.is_some()
            || lists
            || shaped
            || protocol;
        let mut family = None;
        let config = if reconfigures {
            let EntryCredential::ApiKey(config) = &entry.credential else {
                return Err(if patch.models.is_some() {
                    ProviderRefusal::OwnModels(entry.label.clone())
                } else if shaped {
                    ProviderRefusal::ShapeFixed(entry.label.clone())
                } else {
                    ProviderRefusal::SubscriptionOnly
                });
            };
            let stored = config.models.clone();
            let mut config = config.clone();
            if let Some(text) = patch.api_key {
                config.api_key = text.map(key).transpose()?;
            }
            // An entry stays the kind it was added as. A vendor entry's
            // endpoint may be a gateway's that serves its vendor's API, and
            // returns to its vendor's when removed.
            if let Some(base_url) = patch.base_url {
                config.base_url = match (base_url, &config.vendor_id) {
                    (None, Some(vendor_id)) => {
                        let vendor = self
                            .provider_registry
                            .offered_vendor(vendor_id)
                            .await
                            .map_err(|error| ProviderRefusal::CatalogUnavailable(error.to_string()))?;
                        // A vendor no longer offered leaves the entry its
                        // family's default.
                        vendor.as_ref().map(vendor_endpoint).transpose()?.flatten()
                    }
                    (base_url, _) => base_url,
                };
            }
            // A vendor's API takes the headers its vendor documents: its
            // provider sends its own, and its user sets those its family
            // declares. Any other is a gateway's, a compatible entry's.
            if let Some(mut headers) = patch.headers {
                if let Some(problem) = ProviderHeader::problem(&headers) {
                    return Err(ProviderRefusal::Headers(problem));
                }
                if !config.is_compatible() {
                    // A row without a name sends nothing, as a compatible
                    // entry's does.
                    headers.retain(|header| !header.name.is_empty());
                    let declared = self.provider_registry.vendor_headers(&entry);
                    if let Some(header) = headers
                        .iter()
                        .find(|header| !declared.iter().any(|name| name.eq_ignore_ascii_case(&header.name)))
                    {
                        return Err(ProviderRefusal::VendorHeader {
                            entry: entry.label.clone(),
                            header: header.name.clone(),
                        });
                    }
                    // An empty value is a row left empty, which sends none.
                    headers.retain_mut(|header| {
                        header.value = header.value.trim().to_owned();
                        !header.value.is_empty()
                    });
                }
                config.headers = headers;
            }
            if protocol {
                if !config.is_compatible() {
                    return Err(ProviderRefusal::VendorProtocol(entry.label.clone()));
                }
                let name = patch.provider_type.unwrap_or_else(|| entry.family.clone());
                self.api_key_family(&name, patch.wire_api)?;
                config.wire_api = patch.wire_api;
                family = Some(name).filter(|name| *name != entry.family);
            }
            if let Some(models) = patch.models {
                distinct_models(models.as_ref())?;
                config.models = models;
            }
            // A vendor entry makes media as its vendor does.
            if let Some(shape) = audio_shape {
                if !config.is_compatible() {
                    return Err(ProviderRefusal::ShapeFixed(entry.label.clone()));
                }
                if shape.is_some_and(|shape| !AudioShape::CHOSEN.contains(&shape)) {
                    return Err(ProviderRefusal::ShapeFixed(entry.label.clone()));
                }
                config.audio_shape = shape;
            }
            // A model added of a kind the entry does not make is no model of it:
            // only a compatible entry with a shape for the kind lists one.
            //
            // One kept from before the shape was set to None stays, unused and
            // hidden, and comes back with the shape, so a list that still carries
            // it, as an edit of a text model sends, is taken.
            let shapes = MediaShapes {
                image: None,
                video: None,
                audio: config.audio_shape.filter(|_| config.is_compatible()),
            };
            let kept = |model: &ConfiguredModel| {
                stored.iter().flat_map(|models| &models.0).any(|before| before.id == model.id && before.kind == model.kind)
            };
            let unmade = config.models.iter().flat_map(|models| &models.0).find(|model| {
                model.kind.media().is_some_and(|kind| !shapes.makes(kind)) && !kept(model)
            });
            if let Some(model) = unmade.filter(|_| lists) {
                return Err(ProviderRefusal::UnmadeKind {
                    entry: entry.label.clone(),
                    model: model.id.as_str().to_owned(),
                    kind: model.kind,
                });
            }
            Some(config)
        } else {
            None
        };
        if let Some(rotation) = patch.rotation
            && rotation != Rotation::Manual
        {
            let accounts = match &entry.credential {
                EntryCredential::Subscription { .. } => {
                    self.vault.accounts(entry.id.clone()).await?.len()
                }
                EntryCredential::ApiKey(_) => 0,
            };
            if accounts < 2 {
                return Err(ProviderRefusal::RotationNeedsAccounts);
            }
        }
        // A switch belongs to a kind the entry makes, with the shape this
        // change gives it.
        let switch = |kind| patch.media.as_ref().and_then(|media| media.enabled(kind));
        let search = patch.search.clone().unwrap_or_default();
        let switches = EntrySwitches {
            image: switch(MediaKind::Image),
            video: switch(MediaKind::Video),
            audio: switch(MediaKind::Audio),
            web_search: search.web,
            x_search: search.x,
            search_images: search.images,
            search_videos: search.videos,
        };
        // A search switch belongs to a search the entry's vendor has.
        let offered = self.provider_registry.offered_searches(&entry);
        if search.web.is_some() && !offered.web {
            return Err(ProviderRefusal::SearchesNo(entry.label.clone(), "the web"));
        }
        if search.x.is_some() && !offered.x {
            return Err(ProviderRefusal::SearchesNo(entry.label.clone(), "X"));
        }
        if search.images.is_some() && !offered.images {
            return Err(ProviderRefusal::SearchesNo(entry.label.clone(), "images"));
        }
        if search.videos.is_some() && !offered.videos {
            return Err(ProviderRefusal::SearchesNo(entry.label.clone(), "videos"));
        }
        let shapes = match &config {
            Some(config) if config.is_compatible() => MediaShapes { image: None, video: None, audio: config.audio_shape },
            _ => self.provider_registry.media_shapes(&entry),
        };
        if let Some(kind) = MediaKind::ALL.into_iter().find(|kind| switch(*kind).is_some() && !shapes.makes(*kind)) {
            return Err(ProviderRefusal::MakesNo(entry.label.clone(), kind.plural()));
        }
        let cache_ttl = self.cache_ttl(&entry, family.as_deref(), config.as_ref(), patch.cache_ttl)?;
        let retimes = cache_ttl.is_some();
        let through_device = match patch.through_device {
            Some(device) => Some(self.through_device(&entry.owner, &entry.family, &entry.label, device).await?),
            None => None,
        };
        let reroutes = through_device.is_some();
        let label = patch.label.map(|label| label.into_string());
        let rotates_anew = patch.rotation.is_some_and(|rotation| rotation != entry.rotation());
        let updated = self
            .vault
            .update(
                entry.id.clone(),
                label,
                family,
                config,
                patch.rotation,
                switches,
                cache_ttl,
                through_device,
            )
            .await?
            .ok_or(ProviderRefusal::NotFound)?;
        if reconfigures || retimes || reroutes {
            self.provider_registry.invalidate(&entry.id).await?;
        }
        if rotates_anew {
            self.provider_registry.rotation().changed(&entry.id);
        }
        Ok(updated)
    }

    /// Edits the borrowed entry `entry`: its label, and its share key, which
    /// the lender is asked about now. The entry takes the lender's name for
    /// it while it has the name it was added with.
    async fn update_borrowed(
        &self,
        user: &UserId,
        entry: ProviderEntry,
        patch: ProviderPatch,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        let ProviderPatch {
            label,
            share_key,
            api_key: None,
            base_url: None,
            provider_type: None,
            wire_api: None,
            models: None,
            rotation: None,
            media: None,
            search: None,
            cache_ttl: None,
            through_device: None,
            headers: None,
        } = patch
        else {
            return Err(ProviderRefusal::BorrowedFixed(entry.label.clone()));
        };
        let mut label = label.map(Trimmed::into_string);
        let config = match share_key {
            None => None,
            Some(text) => {
                let share = ShareKey::parse(&text).map_err(|refusal| ProviderRefusal::ShareKey(refusal.to_string()))?;
                let EntryCredential::ApiKey(config) = &entry.credential else {
                    return Err(ProviderRefusal::SubscriptionOnly);
                };
                let mut config = config.clone();
                config.base_url = Some(
                    EndpointUrl::try_from(share.url.to_string())
                        .map_err(|_| ProviderRefusal::ShareKey(NotAKey.to_string()))?,
                );
                config.api_key = Some(key(share.token.clone())?);
                let server = server_name(self.public_url.get().map(|url| url.url()));
                let http = ProviderHttp::new(self.provider_registry.http().clone());
                let lent = Borrowed::new(share, entry.label.clone(), server, http)
                    .lent()
                    .await
                    .map_err(ProviderRefusal::ShareKey)?;
                let named = entry.label == BORROWED_LABEL
                    || entry.label.strip_prefix(BORROWED_LABEL).is_some_and(|rest| rest.starts_with(" ("));
                if label.is_none() && named {
                    let entries = self.vault.entries(user.clone()).await?;
                    label = Some(free_label(lent.name, |taken| {
                        entries.iter().any(|other| other.id != entry.id && other.label == taken)
                    }));
                }
                Some(config)
            }
        };
        let reconfigures = config.is_some();
        let updated = self
            .vault
            .update(entry.id.clone(), label, None, config, None, EntrySwitches::default(), None, None)
            .await?
            .ok_or(ProviderRefusal::NotFound)?;
        if reconfigures {
            self.provider_registry.invalidate(&entry.id).await?;
        }
        Ok(updated)
    }

    /// Puts the entries of `user`'s scope in the order `ids` names them,
    /// each once: the order of Settings › Providers, which generated media
    /// try entries in.
    pub async fn order_providers(&self, user: &UserId, ids: Vec<ProviderId>) -> Result<(), ProviderRefusal> {
        if !self.vault.set_order(user, ids).await? {
            return Err(ProviderRefusal::Order);
        }
        Ok(())
    }

    /// Deletes the entry `id` with its accounts, catalog and quota
    /// snapshots; answers the entry as it was.
    pub async fn delete_provider(
        &self,
        starter: &dyn Starter,
        user: &UserId,
        id: &str,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        let entry = self.managed_entry(user, id).await?;
        let _held = self.hold_entry(&entry)?;
        self.vault.delete(&entry).await?;
        // What the starter keeps of the entry goes with it, as a server's
        // share keys with the sessions borrowed with them.
        starter.provider_deleted(&entry).await?;
        self.provider_registry.forget(&entry.id).await?;
        self.cli_installs.forget(&entry.id);
        Ok(entry)
    }

    /// The catalog of every entry `user` infers with, as the product state
    /// carries it (`crates/provider-registry/src/catalog.rs`): each model
    /// with the selection the backend builds from it, each entry with its
    /// health and its media models, and what the small model's Automatic
    /// resolves to. It wakes no Cloud and runs no model; `refresh` waits for
    /// a shared forced refresh of each source.
    pub async fn model_catalog(
        &self,
        user: &UserId,
        refresh: bool,
    ) -> Result<demi_web_api_protocol::providers::ModelCatalog, CatalogError> {
        self.catalog_of(user, Some(refresh)).await
    }

    /// The catalog a page is shown: [`model_catalog`](Self::model_catalog)
    /// with no wait for a source, so no model source, however slow, holds
    /// the page's state back. An entry whose source was never read lists no
    /// models until the cache announces its read as a new catalog.
    pub async fn known_model_catalog(
        &self,
        user: &UserId,
    ) -> Result<demi_web_api_protocol::providers::ModelCatalog, CatalogError> {
        self.catalog_of(user, None).await
    }

    /// The catalog of `user`'s entries: waiting for each source, with
    /// `Some(refresh)`, or as known now.
    async fn catalog_of(
        &self,
        user: &UserId,
        waits: Option<bool>,
    ) -> Result<demi_web_api_protocol::providers::ModelCatalog, CatalogError> {
        let entries = self.vault.entries_for(user).await?;
        let preferences = self.vault.stores().store_of(user).await?.preferences().await?;
        let mut providers = match waits {
            Some(refresh) => self.provider_registry.model_catalog(&entries, refresh).await,
            None => self.provider_registry.known_model_catalog(&entries).await,
        };
        let mut images = self.entry_media_models(user, MediaKind::Image).await?;
        let mut videos = self.entry_media_models(user, MediaKind::Video).await?;
        let mut audio = self.entry_media_models(user, MediaKind::Audio).await?;
        for provider in &mut providers {
            provider.image_models = images.remove(provider.provider_id.as_str()).unwrap_or_default();
            provider.video_models = videos.remove(provider.provider_id.as_str()).unwrap_or_default();
            provider.audio_models = audio.remove(provider.provider_id.as_str()).unwrap_or_default();
        }
        let small_model = demi_provider_registry::small_model::automatic_small_model(&providers, &preferences);
        Ok(demi_web_api_protocol::providers::ModelCatalog { providers, small_model })
    }

    /// Each entry's models of `kind` by entry id, which the model catalog
    /// carries for the settings page.
    pub async fn entry_media_models(
        &self,
        user: &UserId,
        kind: demi_shared_types::MediaKind,
    ) -> Result<std::collections::HashMap<String, Vec<demi_web_api_protocol::providers::EntryMediaModel>>, demi_provider_registry::registry::RegistryError> {
        crate::media::entry_media_models(self, user, kind).await
    }

    /// The models.dev models `query` matches, filled from the vendor of the
    /// entry `entry` of `user`'s scope where it lists them.

    pub async fn search_models(
        &self,
        user: &UserId,
        query: &str,
        entry: Option<&str>,
    ) -> Result<Vec<ModelMatch>, ProviderRefusal> {
        let vendor = match entry {
            Some(id) => match self.scoped_entry(user, id).await?.credential {
                EntryCredential::ApiKey(config) => config.vendor_id,
                EntryCredential::Subscription { .. } => None,
            },
            None => None,
        };
        self.provider_registry
            .vendors()
            .search(query, vendor.as_deref())
            .await
            .map_err(|error| ProviderRefusal::CatalogUnavailable(error.to_string()))
    }

    /// The models.dev model `reference`, `<vendor>/<model id>`, filled as a
    /// search fills it; none when that vendor lists no such model.
    pub async fn models_dev_model(
        &self,
        reference: &str,
    ) -> Result<Option<ModelMatch>, ProviderRefusal> {
        self.provider_registry
            .vendors()
            .model(reference)
            .await
            .map_err(|error| ProviderRefusal::CatalogUnavailable(error.to_string()))
    }

    /// The device an entry of `family`, labeled `label`, connects through,
    /// none for the server: only an entry whose family connects through
    /// devices takes one, and only one of `owner`'s.
    async fn through_device(
        &self,
        owner: &UserId,
        family: &str,
        label: &str,
        device: Option<DeviceId>,
    ) -> Result<Option<DeviceId>, ProviderRefusal> {
        // The server is every entry's.
        let Some(device) = device else {
            return Ok(None);
        };
        if !self.provider_registry.families().connects_through(family) {
            return Err(ProviderRefusal::ConnectsDirectly(label.to_owned()));
        }
        let store = self.vault.stores().store_of(owner).await?;
        let mut devices = store.paired_devices().await?;
        devices.extend(store.managed_device().await?);
        if !devices.iter().any(|owned| owned.id == device) {
            return Err(ProviderRefusal::UnknownDevice);
        }
        Ok(Some(device))
    }

    /// The API-key family `name`, which must speak `wire_api` when the
    /// entry names one.
    /// The prompt cache lifetime an edit of `entry` stores, if it changes:
    /// the one the patch asks for, which only an entry whose protocol names
    /// lifetimes takes; else, when the edit gives the entry another family or
    /// wire, the one it kept if it is a preset of the new protocol, or that
    /// protocol's default.
    ///
    /// `family` and `config` are the edit's, when it changes them.
    fn cache_ttl(
        &self,
        entry: &ProviderEntry,
        new_family: Option<&str>,
        config: Option<&ApiKeyConfig>,
        asked: Option<Option<CacheTtl>>,
    ) -> Result<Option<Option<CacheTtl>>, ProviderRefusal> {
        let stored_wire = match &entry.credential {
            EntryCredential::ApiKey(config) => config.wire_api,
            EntryCredential::Subscription { .. } => None,
        };
        let wire = config.map_or(stored_wire, |config| config.wire_api);
        let family = self.provider_registry.families().get(new_family.unwrap_or(&entry.family));
        let presets = family.and_then(|family| family.cache_ttl_presets(wire));
        if let Some(asked) = asked {
            if let Some(ttl) = asked.as_ref().filter(|_| presets.is_none()) {
                return Err(ProviderRefusal::CacheTtl {
                    entry: entry.label.clone(),
                    ttl: ttl.as_str().to_owned(),
                });
            }
            return Ok(Some(asked));
        }
        if new_family.is_none() && wire == stored_wire {
            return Ok(None);
        }
        // A lifetime typed for one protocol is not one the next is known to
        // take: only a preset of the new protocol carries over.
        let known = |ttl: &CacheTtl| presets.as_ref().is_some_and(|presets| presets.contains(ttl));
        let next = entry.cache_ttl.clone().filter(|ttl| known(ttl)).or_else(|| {
            family
                .and_then(|family| family.default_cache_ttl())
                .filter(|ttl| known(ttl))
        });
        Ok((next != entry.cache_ttl).then_some(next))
    }

    fn api_key_family(
        &self,
        name: &str,
        wire_api: Option<WireApi>,
    ) -> Result<&Arc<dyn ProviderFamily>, ProviderRefusal> {
        let family = self
            .provider_registry
            .families()
            .get(name)
            .ok_or_else(|| ProviderRefusal::UnknownFamily(name.to_owned()))?;
        if family.credential() == CredentialKind::Subscription {
            return Err(ProviderRefusal::LoginFamily(name.to_owned()));
        }
        if let Some(wire) = wire_api.filter(|wire| !family.wires().contains(wire)) {
            return Err(ProviderRefusal::Wire {
                family: name.to_owned(),
                wire,
            });
        }
        Ok(family)
    }
}

/// The endpoint an entry of `vendor` starts with and returns to; none for a
/// vendor that names none, whose entry goes to its family's default.
fn vendor_endpoint(vendor: &Vendor) -> Result<Option<EndpointUrl>, ProviderRefusal> {
    vendor.base_url.clone().map(EndpointUrl::try_from).transpose().map_err(|_| {
        ProviderRefusal::CatalogUnavailable(format!("models.dev names no usable endpoint for {}", vendor.id))
    })
}

/// A key as the vault keeps it: one line of text.
fn key(text: String) -> Result<Secret, ProviderRefusal> {
    Ok(Secret::try_from(text)?)
}

/// The family a compatible entry speaks when it names none: Anthropic
/// Messages.
const COMPATIBLE_FAMILY: &str = "anthropic";

/// What a borrowed entry is named until its share key names it after the
/// lender's.
const BORROWED_LABEL: &str = "Another Demi";

/// The name a compatible entry takes when it is given none.
const COMPATIBLE_LABEL: &str = "Compatible API";

/// Refuses a model list that names one id twice.
fn distinct_models(models: Option<&ConfiguredModels>) -> Result<(), ProviderRefusal> {
    match models.and_then(ConfiguredModels::repeated_id) {
        Some(id) => Err(ProviderRefusal::DuplicateModel(id.to_owned())),
        None => Ok(()),
    }
}

/// `name`, or the first of `name (2)`, `name (3)`, … that `taken` does not
/// hold.
pub(crate) fn free_label(name: String, taken: impl Fn(&str) -> bool) -> String {
    if !taken(&name) {
        return name;
    }
    (2..)
        .map(|number| format!("{name} ({number})"))
        .find(|label| !taken(label))
        .expect("a scope holds finitely many entries")
}

/// Why the model catalog could not be read.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Registry(#[from] demi_provider_registry::registry::RegistryError),
}
