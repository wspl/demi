//! The changes of the provider entries in a user's scope, and the model search
//! that fills a configured model
//! (`crates/backend-providers/src/llm/vendors.rs`).
//!
//! There is one path, with its checks and its words, for the settings page and
//! for `demi providers` (`crates/backend-user-shard/src/shard/provider_commands.rs`).
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

use demi_backend_database::StorageError;
use demi_backend_database::providers::MediaSwitches;
use demi_backend_providers::llm::families::{ProviderFamily, family_name};
use demi_backend_providers::vault::entries::{ApiKeyConfig, EntryCredential, ProviderEntry};
use demi_backend_providers::vault::operations::OperationGuard;
use demi_provider_common::{Secret, SecretError};
use demi_shared_types::{AudioShape, ImageShape, MediaKind, MediaShapes, VideoShape, WireApi};
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::ids::{ProviderId, UserId};
use demi_web_api_protocol::providers::{
    ConfiguredModel, ConfiguredModels, CreateProvider, CredentialKind, ModelKind, ModelMatch, ProviderPatch, Rotation,
};
use demi_web_api_protocol::text::EndpointUrl;
use std::sync::Arc;

use crate::services::Services;

/// Why a change of an entry, or a search, was refused, in the words both
/// the settings page and `demi providers` show.
#[derive(Debug, thiserror::Error)]
pub enum ProviderRefusal {
    /// The user only infers with the scope's entries.
    #[error("Providers are configured by the instance owner")]
    NotConfiguring,
    #[error("No such provider")]
    NotFound,
    #[error("Another change of this provider is still running")]
    Busy,
    #[error("Unknown provider type \"{0}\"")]
    UnknownFamily(String),
    /// A subscription family, whose entry a login makes.
    #[error("Provider type \"{0}\" is configured by its login")]
    LoginFamily(String),
    #[error("wireApi: the {family} family does not speak {wire}")]
    Wire { family: String, wire: WireApi },
    #[error("Unknown vendor \"{0}\"")]
    UnknownVendor(String),
    /// The models.dev document could not be read.
    #[error("{0}")]
    CatalogUnavailable(String),
    #[error("apiKey: {0}")]
    Key(#[from] SecretError),
    /// A key or an endpoint for a subscription entry.
    #[error("A subscription entry takes only a new label")]
    SubscriptionOnly,
    /// A model list for a subscription entry, whose models come from its
    /// provider; it names the entry.
    #[error("{0} lists its own models; its list cannot be edited.")]
    OwnModels(String),
    /// An endpoint for a vendor entry, which uses its vendor's; it names the
    /// entry.
    #[error("{0} uses its vendor’s endpoint; it can’t be changed.")]
    VendorEndpoint(String),
    /// A protocol for a vendor entry, which speaks its vendor's; it names
    /// the entry.
    #[error("{0} speaks its vendor’s protocol; it can’t be changed.")]
    VendorProtocol(String),
    /// A rotation other than manual for an entry with fewer than two
    /// accounts.
    #[error("Rotation needs at least two accounts")]
    RotationNeedsAccounts,
    /// A switch of a kind for an entry that makes none of it; it names the
    /// entry and the kind.
    #[error("{0} doesn’t make {1}.")]
    MakesNo(String, &'static str),
    /// A shape for a vendor entry, whose shapes are its vendor's; it names
    /// the entry.
    #[error("{0} makes media as its vendor does; its shapes can’t be changed.")]
    ShapeFixed(String),
    /// A model of a kind the entry makes none of.
    #[error("models: {model} is {} {kind} model, but {entry} makes no {}; choose {} {kind} shape first", article(*kind), plural(*kind), article(*kind))]
    UnmadeKind { entry: String, model: String, kind: ModelKind },
    /// A list of entries for an order that does not name each of the
    /// scope's once.
    #[error("providerIds: name every provider once")]
    Order,
    /// A model list that names one id twice; it names the id.
    #[error("models: the model ID {0:?} appears twice")]
    DuplicateModel(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// The article before a kind of model: an image model, a video model, an
/// audio model.
fn article(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Image | ModelKind::Audio => "an",
        ModelKind::Text | ModelKind::Video => "a",
    }
}

/// What an entry makes of a kind of model, as a sentence names it: images,
/// videos, audio.
fn plural(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Text => "text",
        ModelKind::Image => "images",
        ModelKind::Video => "videos",
        ModelKind::Audio => "audio",
    }
}

impl Services {
    /// Refuses a user who does not configure the entries of their scope.
    pub fn may_configure(&self, user: &UserDto) -> Result<(), ProviderRefusal> {
        if self.vault.configures(user) {
            Ok(())
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

    /// Creates an API-key entry: a vendor entry from the vendor list, or from a
    /// family alone for its own vendor; or a compatible entry, Anthropic Messages
    /// unless the body names another protocol.
    ///
    /// Its label is the vendor's or the family's name, or *Compatible API*, unless
    /// the body gives one. What the body leaves out leaves it incomplete.
    pub async fn create_provider(
        &self,
        user: &UserDto,
        body: CreateProvider,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        self.may_configure(user)?;
        let (family, label, config) = match body {
            CreateProvider::Vendor {
                vendor_id,
                label,
                api_key,
                models,
            } => {
                let vendor = self
                    .assembly
                    .offered_vendor(&vendor_id)
                    .await
                    .map_err(|error| ProviderRefusal::CatalogUnavailable(error.to_string()))?
                    .ok_or(ProviderRefusal::UnknownVendor(vendor_id))?;
                self.api_key_family(&vendor.provider_type, vendor.wire_api)?;
                let base_url = vendor
                    .base_url
                    .map(EndpointUrl::try_from)
                    .transpose()
                    .map_err(|_| {
                        ProviderRefusal::CatalogUnavailable(format!(
                            "models.dev names no usable endpoint for {}",
                            vendor.id
                        ))
                    })?;
                let label = label.ok_or(vendor.name);
                let config = ApiKeyConfig {
                    api_key: api_key.map(key).transpose()?,
                    base_url,
                    wire_api: vendor.wire_api,
                    vendor_id: Some(vendor.id),
                    models,
                    image_shape: None,
                    video_shape: None,
                    audio_shape: None,
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
                    image_shape: None,
                    video_shape: None,
                    audio_shape: None,
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
                    image_shape: None,
                    video_shape: None,
                    audio_shape: None,
                };
                (provider_type, label, config)
            }
        };
        distinct_models(config.models.as_ref())?;
        let owner = self.vault.owner_for(&user.id).await?;
        let label = match label {
            Ok(given) => given.into_string(),
            Err(default) => {
                let entries = self.vault.entries(owner.clone()).await?;
                free_label(default, |taken| entries.iter().any(|entry| entry.label == taken))
            }
        };
        Ok(self
            .vault
            .create_api_key(owner, family, label, config)
            .await?)
    }

    /// Edits the entry `id`: the label of any; the key and model list of an
    /// API-key entry; and a compatible entry's endpoint and protocol, whose
    /// change of family keeps its key, endpoint and models. A configuration
    /// edit starts the entry's provider and catalog afresh.
    pub async fn update_provider(
        &self,
        user: &UserDto,
        id: &str,
        patch: ProviderPatch,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        self.may_configure(user)?;
        let entry = self.scoped_entry(&user.id, id).await?;
        let _held = self.hold_entry(&entry)?;
        let protocol = patch.provider_type.is_some() || patch.wire_api.is_some();
        let image_shape = patch.media.as_ref().and_then(|media| media.image.as_ref()).and_then(|image| image.shape);
        let video_shape = patch.media.as_ref().and_then(|media| media.video.as_ref()).and_then(|video| video.shape);
        let audio_shape = patch.media.as_ref().and_then(|media| media.audio.as_ref()).and_then(|audio| audio.shape);
        let shaped = image_shape.is_some() || video_shape.is_some() || audio_shape.is_some();
        let lists = patch.models.is_some();
        let reconfigures = patch.api_key.is_some()
            || patch.base_url.is_some()
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
            // An entry stays the kind it was added as: a vendor's endpoint
            // and protocol are its vendor's.
            if let Some(base_url) = patch.base_url {
                if !config.is_compatible() {
                    return Err(ProviderRefusal::VendorEndpoint(entry.label.clone()));
                }
                config.base_url = base_url;
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
            if let Some(shape) = image_shape {
                if !config.is_compatible() {
                    return Err(ProviderRefusal::ShapeFixed(entry.label.clone()));
                }
                if shape.is_some_and(|shape| !ImageShape::CHOSEN.contains(&shape)) {
                    return Err(ProviderRefusal::ShapeFixed(entry.label.clone()));
                }
                config.image_shape = shape;
            }
            if let Some(shape) = video_shape {
                if !config.is_compatible() {
                    return Err(ProviderRefusal::ShapeFixed(entry.label.clone()));
                }
                if shape.is_some_and(|shape| !VideoShape::CHOSEN.contains(&shape)) {
                    return Err(ProviderRefusal::ShapeFixed(entry.label.clone()));
                }
                config.video_shape = shape;
            }
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
                image: config.image_shape.filter(|_| config.is_compatible()),
                video: config.video_shape.filter(|_| config.is_compatible()),
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
        let switches = MediaSwitches {
            image: switch(MediaKind::Image),
            video: switch(MediaKind::Video),
            audio: switch(MediaKind::Audio),
        };
        let shapes = match &config {
            Some(config) if config.is_compatible() => MediaShapes {
                image: config.image_shape,
                video: config.video_shape,
                audio: config.audio_shape,
            },
            _ => self.assembly.media_shapes(&entry),
        };
        if let Some(kind) = MediaKind::ALL.into_iter().find(|kind| switch(*kind).is_some() && !shapes.makes(*kind)) {
            return Err(ProviderRefusal::MakesNo(entry.label.clone(), kind.plural()));
        }
        let label = patch.label.map(|label| label.into_string());
        let updated = self
            .vault
            .update(
                entry.id.clone(),
                label,
                family,
                config,
                patch.rotation,
                switches,
            )
            .await?
            .ok_or(ProviderRefusal::NotFound)?;
        if reconfigures {
            self.assembly.invalidate(&entry.id).await?;
        }
        Ok(updated)
    }

    /// Puts the entries of `user`'s scope in the order `ids` names them,
    /// each once: the order of Settings › Providers, which generated media
    /// try entries in.
    pub async fn order_providers(&self, user: &UserDto, ids: Vec<ProviderId>) -> Result<(), ProviderRefusal> {
        self.may_configure(user)?;
        let owner = self.vault.owner_for(&user.id).await?;
        if !self.vault.set_order(&owner, ids).await? {
            return Err(ProviderRefusal::Order);
        }
        Ok(())
    }

    /// Deletes the entry `id` with its accounts, catalog and quota
    /// snapshots; answers the entry as it was.
    pub async fn delete_provider(
        &self,
        user: &UserDto,
        id: &str,
    ) -> Result<ProviderEntry, ProviderRefusal> {
        self.may_configure(user)?;
        let entry = self.scoped_entry(&user.id, id).await?;
        let _held = self.hold_entry(&entry)?;
        self.vault.delete(&entry).await?;
        self.assembly.forget(&entry.id).await?;
        self.cli_installs.forget(&entry.id);
        Ok(entry)
    }

    /// Each entry's models of `kind` by entry id, which `GET /api/models`
    /// carries for the settings page.
    pub async fn entry_media_models(
        &self,
        user: &UserId,
        kind: demi_shared_types::MediaKind,
    ) -> Result<std::collections::HashMap<String, Vec<demi_web_api_protocol::providers::EntryMediaModel>>, demi_backend_providers::llm::assembly::AssemblyError> {
        crate::shard::media::entry_media_models(self, user, kind).await
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
        self.assembly
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
        self.assembly
            .vendors()
            .model(reference)
            .await
            .map_err(|error| ProviderRefusal::CatalogUnavailable(error.to_string()))
    }

    /// The API-key family `name`, which must speak `wire_api` when the
    /// entry names one.
    fn api_key_family(
        &self,
        name: &str,
        wire_api: Option<WireApi>,
    ) -> Result<&Arc<dyn ProviderFamily>, ProviderRefusal> {
        let family = self
            .assembly
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

/// A key as the vault keeps it: one line of text.
fn key(text: String) -> Result<Secret, ProviderRefusal> {
    Ok(Secret::try_from(text)?)
}

/// The family a compatible entry speaks when it names none: Anthropic
/// Messages.
const COMPATIBLE_FAMILY: &str = "anthropic";

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
fn free_label(name: String, taken: impl Fn(&str) -> bool) -> String {
    if !taken(&name) {
        return name;
    }
    (2..)
        .map(|number| format!("{name} ({number})"))
        .find(|label| !taken(label))
        .expect("a scope holds finitely many entries")
}
