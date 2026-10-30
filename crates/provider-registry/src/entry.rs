//! A provider entry, read and decoded: what its family builds its provider
//! from, and how it authenticates.

use demi_provider_common::{Searches, Secret};
use demi_shared_types::{AudioShape, CacheTtl, ImageShape, MediaKind, MediaShapes, Timestamp, VideoShape, WireApi};
use demi_web_api_protocol::ids::{CredentialId, ProviderId, UserId};
use demi_web_api_protocol::providers::{
    AudioMedia, ConfiguredModels, CredentialKind, EntryMedia, EntrySearch, ImageMedia, ProviderDto, Rotation, SetupPart, VideoMedia,
};
use demi_web_api_protocol::text::EndpointUrl;
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

/// A provider entry, read and decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderEntry {
    pub id: ProviderId,
    pub owner: UserId,
    /// The entry's family, such as `anthropic` or `codex`.
    pub family: String,
    pub label: String,
    pub credential: EntryCredential,
    pub created_at: Timestamp,
    /// Whether the entry makes images when its provider can: its Image
    /// Generation switch (`crates/backend-user-shard/src/provider_settings.rs`).
    pub image_generation: bool,
    /// Whether the entry makes videos when its provider can: its Video
    /// Generation switch.
    pub video_generation: bool,
    /// Whether the entry makes audio when its provider can: its Audio
    /// Generation switch.
    pub audio_generation: bool,
    /// Whether its models use its vendor's web search, when the vendor has
    /// one: its Web Search switch, on from the start.
    pub web_search: bool,
    /// Whether they use its vendor's X search, when it has one: its X Search
    /// switch.
    pub x_search: bool,
    /// Whether those searches look at images and watch videos, when the
    /// vendor's can: its Images and Videos switches, on from the start.
    pub search_images: bool,
    pub search_videos: bool,
    /// The prompt cache lifetime its requests ask for; none to send none.
    pub cache_ttl: Option<CacheTtl>,
}

/// How an entry authenticates.
#[derive(Debug, Clone, PartialEq)]
pub enum EntryCredential {
    ApiKey(ApiKeyConfig),
    /// Accounts in the entry's credential records, of which `active` is the
    /// one the entry infers with unless `rotation` gives each agent one of
    /// its own (`crates/provider-registry/src/rotation.rs`).
    Subscription {
        active: Option<CredentialId>,
        rotation: Rotation,
    },
}

/// An API-key entry's configuration: the document a program stores sealed. A field
/// it does not name is an error, so a misspelled setting is reported rather
/// than ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiKeyConfig {
    /// None until the user saves one: the entry is incomplete meanwhile.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub api_key: Option<Secret>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub base_url: Option<EndpointUrl>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub wire_api: Option<WireApi>,
    /// The vendor of a vendor entry: one models.dev lists, or its family's
    /// own, whose models.dev id is the family's name (`anthropic`,
    /// `openai`, `google`). A compatible entry names none.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(length(min = 1)))]
    pub vendor_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(dive)]
    pub models: Option<ConfiguredModels>,
    /// A compatible entry's image shape, which its user chose; none until
    /// then, and always none for a vendor entry, whose shape is its
    /// vendor's.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub image_shape: Option<ImageShape>,
    /// A compatible entry's video shape, which its user chose, as its image
    /// shape is.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub video_shape: Option<VideoShape>,
    /// A compatible entry's audio shape, which its user chose, as its image
    /// shape is.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub audio_shape: Option<AudioShape>,
}

impl ApiKeyConfig {
    /// Whether the entry is a **compatible entry**
    /// (`crates/provider-common/src/endpoint.rs`): it names no vendor.
    ///
    /// - A vendor entry uses its vendor's endpoint and lists its vendor's
    ///   catalog.
    /// - A compatible entry has the endpoint the user gives, none until then,
    ///   and lists only the models the user configures.
    pub fn is_compatible(&self) -> bool {
        self.vendor_id.is_none()
    }
}

impl ProviderEntry {
    pub fn kind(&self) -> CredentialKind {
        match self.credential {
            EntryCredential::ApiKey(_) => CredentialKind::ApiKey,
            EntryCredential::Subscription { .. } => CredentialKind::Subscription,
        }
    }

    /// What the entry lacks to infer, in the order the page lists it
    /// (`crates/backend-user-shard/src/provider_settings.rs`):
    ///
    /// - A vendor entry's key.
    /// - A compatible entry's endpoint. Its key is optional.
    /// - A model, when its catalog is its configured list and the list is
    ///   empty. A compatible entry's catalog always is its configured list.
    /// - A subscription's account.
    ///
    /// A vendor entry's address is never missing: it uses its vendor's. The
    /// result is empty for a complete entry.
    pub fn missing(&self) -> Vec<SetupPart> {
        match &self.credential {
            EntryCredential::ApiKey(config) => [
                (
                    !config.is_compatible() && config.api_key.is_none(),
                    SetupPart::ApiKey,
                ),
                (
                    config.is_compatible() && config.base_url.is_none(),
                    SetupPart::Endpoint,
                ),
                (
                    config
                        .models
                        .as_ref()
                        .map_or(config.is_compatible(), |models| models.0.is_empty()),
                    SetupPart::Models,
                ),
            ]
            .into_iter()
            .filter_map(|(lacks, part)| lacks.then_some(part))
            .collect(),
            EntryCredential::Subscription { active, .. } => match active {
                Some(_) => Vec::new(),
                None => vec![SetupPart::Account],
            },
        }
    }

    /// A subscription entry's active account.
    pub fn active(&self) -> Option<&CredentialId> {
        match &self.credential {
            EntryCredential::ApiKey(_) => None,
            EntryCredential::Subscription { active, .. } => active.as_ref(),
        }
    }

    /// Which account each agent uses; `manual` for an API-key entry.
    pub fn rotation(&self) -> Rotation {
        match &self.credential {
            EntryCredential::ApiKey(_) => Rotation::Manual,
            EntryCredential::Subscription { rotation, .. } => *rotation,
        }
    }

    /// The vendor's searches its models use: those `offered` that the user
    /// turned on.
    pub fn searches(&self, offered: Searches) -> Searches {
        let web = offered.web && self.web_search;
        let x = offered.x && self.x_search;
        Searches {
            web,
            x,
            images: offered.images && self.search_images && (web || x),
            videos: offered.videos && self.search_videos && x,
        }
    }

    /// The switch of `kind`.
    pub fn generates(&self, kind: MediaKind) -> bool {
        match kind {
            MediaKind::Image => self.image_generation,
            MediaKind::Video => self.video_generation,
            MediaKind::Audio => self.audio_generation,
        }
    }

    /// The entry as the web app sees it: never its key.
    ///
    /// - `default_endpoint` is its family's default base URL, which a vendor
    ///   entry that stores no endpoint goes to. A compatible entry without one
    ///   goes nowhere yet.
    /// - `shapes` are the ones it makes media in.
    /// - `catalog` is when the media catalog in use was published.
    /// - `offered` are the searches its vendor has.
    #[allow(clippy::too_many_arguments)]
    pub fn dto(
        &self,
        default_endpoint: Option<EndpointUrl>,
        shapes: MediaShapes,
        offered: Searches,
        catalog: Timestamp,
        cache_ttl_presets: Option<Vec<CacheTtl>>,
    ) -> ProviderDto {
        let config = match &self.credential {
            EntryCredential::ApiKey(config) => Some(config),
            EntryCredential::Subscription { .. } => None,
        };
        ProviderDto {
            id: self.id.clone(),
            kind: self.kind(),
            provider_type: self.family.clone(),
            label: self.label.clone(),
            wire_api: config.and_then(|config| config.wire_api),
            vendor_id: config.and_then(|config| config.vendor_id.clone()),
            compatible: config.is_some_and(ApiKeyConfig::is_compatible),
            endpoint: config.and_then(|config| match config.is_compatible() {
                true => config.base_url.clone(),
                false => config.base_url.clone().or(default_endpoint),
            }),
            has_api_key: config.is_some_and(|config| config.api_key.is_some()),
            models: config.and_then(|config| config.models.clone()),
            rotation: self.rotation(),
            media: EntryMedia {
                image: ImageMedia {
                    shape: shapes.image,
                    enabled: self.image_generation,
                },
                video: VideoMedia {
                    shape: shapes.video,
                    enabled: self.video_generation,
                },
                audio: AudioMedia {
                    shape: shapes.audio,
                    enabled: self.audio_generation,
                },
                catalog,
            },
            search: EntrySearch {
                web: offered.web.then_some(self.web_search),
                x: offered.x.then_some(self.x_search),
                images: offered.images.then_some(self.search_images),
                videos: offered.videos.then_some(self.search_videos),
            },
            cache_ttl: self.cache_ttl.clone(),
            cache_ttl_presets,
            created_at: self.created_at,
        }
    }
}
