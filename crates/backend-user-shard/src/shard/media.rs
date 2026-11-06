//! The Providers service of the plugins' port for generated media
//! (`crates/plugin-interface/src/lib.rs`).
//!
//! The service offers:
//!
//! - the models of one kind the user's entries offer, each with every fact the
//!   catalog of media models states and the entries that offer it;
//! - one image made through an entry's account or key;
//! - one video's job, from its ask to its download;
//! - one audio file, with the voices a speech model speaks in.
//!
//! # Which entry makes a call
//!
//! - The one `--provider` names, which fails the call when it cannot.
//! - Otherwise the first that can make the model now, the subscriptions before
//!   the keys, each in the order of Settings › Providers.
//!
//! An entry can when:
//!
//! - the user has it on and its switch for the kind is on;
//! - it is signed in or has its key;
//! - it offers the model.
//!
//! An entry whose every account shows used up comes after the others, the
//! soonest to reset first, and is asked all the same: only its vendor refuses.
//!
//! # Handing a call on
//!
//! - An entry whose last account is used up by the call's request hands it on to
//!   the next.
//! - A refusal of the request is never handed on, and neither is a video's job
//!   once the vendor took it, since it is paid for.
//! - A video asked of an option the model's catalog entry says it does not
//!   take fails at the first entry that offers the model, before anything is
//!   sent, as every entry of the model sends the same options.
//!
//! # The call
//!
//! Within an entry the call takes the account under the entry's rotation, as a
//! new agent takes one at its first request, and another at once when that one
//! is used up. The provider sends the request, renewing its account's sign-in,
//! and each call is a usage ledger row with the purpose of its kind. What the
//! agent reads of it, the plugin words.

use std::rc::Rc;
use std::sync::Arc;

use bytes::Bytes;
use demi_core_store::media_jobs::{Follow, JobOfCall, Made, MediaJob, MediaJobStatus};
use demi_core_store::usage::{MadeVideoCall, UsageRow, merge_windows};
use demi_core_providers::vault::entries::Vault;
use demi_plugin_host::{MediaCall, VideoJob};
use demi_provider_registry::store::StoreError;
use demi_provider_registry::registry::{RegistryError, MakerOf};
use demi_provider_registry::rotation::{Choice, USED_UP_FOR_MS};
use demi_provider_registry::entry::{EntryCredential, ProviderEntry};
use demi_core_host_access::root_of;
use demi_host_interface::{JobCaller, PortError};
use demi_plugin_interface::{
    AskedVideo, AudioAsk, AudioFacts, AudioFileFormat, AudioOption, AudioVoices, MadeAudio, PcmShape, VoiceInfo,
    CustomSizeRule, EntryStatus, ImageAsk, ImageFacts, MadeImage, MadeVideo, MediaListing, MediaModel,
    MediaPrice, MediaRefusal, MediaUnlisted, ModelEntry, PortFailure, PortRefusal, PriceRate, SentSource, Unable,
    VideoAsk, VideoChange, VideoFacts,
};
use demi_provider_common::ErrorCode;
use demi_provider_common::credentials::AuthReason;
use demi_provider_common::audio::{self, AudioFormatAsk, AudioRequest};
use demi_provider_common::images::{Aspect, ImageRequest, InputImage};
use demi_provider_common::media::{MediaFailure, MediaModelInfo, MediaVendor};
use demi_provider_common::media_catalog::{
    self, AudioOutput, Catalog, Currency, Modality, PriceUnit, VideoCapabilities, Voices,
};
use demi_provider_common::videos::{self, Change, SourceVideo, VideoMaker, VideoRequest, VideoSource};
use demi_shared_types::{AudioKind, MediaKind, Model, QuotaWindow, Rotation, Timestamp, TurnId, VendorTrace};
use demi_web_api_protocol::ids::{ConversationId, CredentialId, ProviderId};
use demi_web_api_protocol::providers::EntryMediaModel;
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::Shard;
use super::media_jobs::Reader;
use super::provider_commands::named;
use crate::services::Services;

/// An entry that makes media of a kind, with what makes them for its
/// active account, in the order a call tries the entries.
pub(super) struct Route {
    entry: ProviderEntry,
    kind: MediaKind,
    pub(super) maker: MakerOf,
    /// The user turned the entry off in Settings › Providers.
    hidden: bool,
}

impl Route {
    fn media(&self) -> &dyn demi_provider_common::media::MediaMaker {
        self.maker.media()
    }

    /// The client the entry's calls go through on `client`, the shard's or
    /// the shared services'.
    pub(super) fn http(&self, services: &Services, client: &reqwest::Client) -> demi_provider_common::ProviderHttp {
        services.provider_registry.http_for(&self.entry, client.clone())
    }

    fn subscription(&self) -> bool {
        matches!(self.entry.credential, EntryCredential::Subscription { .. })
    }
}

/// Whether an entry would make a call of a model now, judged by all but
/// whether it offers the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    Ready,
    UsedUp(Option<Timestamp>),
    Off,
    NotSignedIn,
    TurnedOff,
}

impl Standing {
    fn unable(self) -> Option<Unable> {
        match self {
            Self::Ready => None,
            Self::UsedUp(until) => Some(Unable::UsedUp { until }),
            Self::Off => Some(Unable::Off),
            Self::NotSignedIn => Some(Unable::NotSignedIn),
            Self::TurnedOff => Some(Unable::TurnedOff),
        }
    }
}

fn refused(refusal: MediaRefusal) -> PortFailure {
    PortFailure::Refused(PortRefusal::Media { refusal })
}

fn failed(error: impl ToString) -> PortFailure {
    refused(MediaRefusal::Failed { message: error.to_string() })
}

/// The user's entries that make `kind`, in the order a call tries them:
/// the subscriptions before the keys, each in the order of Settings ›
/// Providers. An entry that lacks what its provider is built from, such as
/// a key, offers nothing yet and is left out.
async fn routes(
    services: &Services,
    user: &demi_web_api_protocol::ids::UserId,
    kind: MediaKind,
) -> Result<Vec<Route>, RegistryError> {
    let store = services.stores.of(user).await.map_err(StoreError::new)?;
    let preferences = store.preferences().await.map_err(StoreError::new)?;
    let mut routes = Vec::new();
    for entry in services.vault.entries_for(user).await.map_err(StoreError::new)? {
        if !services.provider_registry.makes(&entry, kind) {
            continue;
        }
        let Some(route) = route(services, entry, &preferences, kind).await? else {
            continue;
        };
        routes.push(route);
    }
    // A stable sort keeps the order of Settings › Providers within each
    // kind of entry.
    routes.sort_by_key(|route| !route.subscription());
    Ok(routes)
}

pub(super) async fn route(
    services: &Services,
    entry: ProviderEntry,
    preferences: &demi_web_api_protocol::settings::Preferences,
    kind: MediaKind,
) -> Result<Option<Route>, RegistryError> {
    let provider = match services.provider_registry.provider_for(&entry).await {
        Ok(provider) => provider,
        // An entry that lacks what it needs makes nothing yet.
        Err(RegistryError::Incomplete) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(maker) = services.provider_registry.maker(&entry, &provider, kind) else {
        return Ok(None);
    };
    Ok(Some(Route {
        hidden: preferences.hides_provider(&entry.id),
        entry,
        kind,
        maker,
    }))
}

/// Entry `route` as it is now, while it may still make its kind.
///
/// An entry may when:
///
/// - it exists;
/// - the user has it on and its switch for the kind is on;
/// - it is signed in or has its key.
///
/// It is read again right before a request is sent, since any of them may have
/// changed while an earlier entry's request was under way.
async fn current(
    services: &Services,
    user: &demi_web_api_protocol::ids::UserId,
    route: &Route,
) -> Result<Option<Route>, RegistryError> {
    let Some(entry) = services.vault.entry(route.entry.id.clone()).await.map_err(StoreError::new)? else {
        return Ok(None);
    };
    let store = services.stores.of(user).await.map_err(StoreError::new)?;
    let preferences = store.preferences().await.map_err(StoreError::new)?;
    let Some(now) = self::route(services, entry, &preferences, route.kind).await? else {
        return Ok(None);
    };
    let usable = !now.hidden
        && now.entry.generates(route.kind)
        && !matches!(&now.entry.credential, EntryCredential::Subscription { active: None, .. });
    Ok(usable.then_some(now))
}

/// The model `id` as a rotation reads a window's scope.
pub(super) fn model_of(id: &str) -> Model {
    Model {
        id: id.to_owned(),
        name: id.to_owned(),
        context_window: 0,
        output_limit: None,
        thinking: Vec::new(),
        accepted_extensions: None,
    }
}

/// How `route` stands for a call of `model` now. A key's limit is known
/// only from a call's refusal; a subscription's accounts are judged as the
/// entry's rotation judges them, counting none.
async fn standing(services: &Services, route: &Route, model: &str) -> Result<Standing, RegistryError> {
    let entry = &route.entry;
    if route.hidden {
        return Ok(Standing::TurnedOff);
    }
    if !entry.generates(route.kind) {
        return Ok(Standing::Off);
    }
    if !route.subscription() {
        return Ok(Standing::Ready);
    }
    let Some(active) = entry.active() else {
        return Ok(Standing::NotSignedIn);
    };
    let provider_registry = &services.provider_registry;
    let mut candidates = provider_registry.candidates(entry, false).await?;
    if entry.rotation() == Rotation::Manual {
        candidates.retain(|candidate| &candidate.id == active);
    }
    // An entry whose accounts its vendor all signed out has none to use.
    if candidates.is_empty() {
        return Ok(Standing::NotSignedIn);
    }
    let now = provider_registry.now();
    // A mark a call left counts as a window does, with its moment.
    Ok(match provider_registry.rotation().all_used_up(&entry.id, &candidates, &model_of(model), now) {
        Some(until) => Standing::UsedUp(until),
        None => Standing::Ready,
    })
}

/// Each entry's models of `kind`, by entry id, newest first, for the settings
/// page's Images, Videos and Audio sections
/// (`crates/backend-user-shard/src/provider_settings.rs`).
///
/// An entry is listed whether or not its switch for the kind is on, and an alias
/// is left out beside the model it names. An entry whose models could not be
/// listed has none here; the agent's listing says why.
pub async fn entry_media_models(
    services: &Services,
    user: &demi_web_api_protocol::ids::UserId,
    kind: MediaKind,
) -> Result<std::collections::HashMap<String, Vec<EntryMediaModel>>, RegistryError> {
    let routes = routes(services, user, kind).await?;
    let catalog = services.media.current();
    let https: Vec<_> = routes.iter().map(|route| route.http(services, services.provider_registry.http())).collect();
    let listings =
        futures_util::future::join_all(routes.iter().zip(&https).map(|(route, http)| route.media().models(http, &catalog))).await;
    let mut entries = std::collections::HashMap::new();
    for (route, listing) in routes.iter().zip(listings) {
        let listed = listing.unwrap_or_default();
        let vendors: Vec<MediaVendor> = route.media().vendor().into_iter().collect();
        let mut models: Vec<EntryMediaModel> = listed
            .iter()
            .filter(|model| {
                let named = vendors.iter().find_map(|vendor| catalog.model(vendor.id(), &model.id));
                !named
                    .and_then(|known| known.alias_of.as_deref())
                    .is_some_and(|target| listed.iter().any(|other| other.id == target))
            })
            .map(|model| {
                let described = described(&catalog, kind, model, &vendors, Vec::new());
                EntryMediaModel {
                    id: described.id,
                    name: described.name.unwrap_or_else(|| model.id.clone()),
                    released: described.released,
                    kinds: described.audio.map(|audio| audio.kinds).unwrap_or_default(),
                }
            })
            .collect();
        models.sort_by(|a, b| b.released.cmp(&a.released));
        entries.insert(route.entry.id.as_str().to_owned(), models);
    }
    Ok(entries)
}

/// The active account's label of `route`'s entry, for a user who manages
/// it.
async fn account_of(shard: &Shard, route: &Route) -> Option<String> {
    let active = route.entry.active().filter(|_| Vault::manages(shard.user(), &route.entry))?;
    let record = shard.services().vault.account(route.entry.id.clone(), active.clone()).await.ok().flatten();
    Some(record.map_or_else(|| active.to_string(), |record| record.label))
}

/// Each route's models, in the routes' order; a route whose models could
/// not be listed offers none and is told in `unlisted`.
async fn offerings(
    shard: &Shard,
    catalog: &Catalog,
    routes: &[Route],
) -> (Vec<Vec<MediaModelInfo>>, Vec<MediaUnlisted>, Option<(String, MediaFailure, bool)>) {
    let mut offered = Vec::new();
    let mut unlisted = Vec::new();
    let mut first_failure = None;
    for route in routes {
        match route.media().models(&route.http(shard.services(), shard.http()), catalog).await {
            Ok(models) => offered.push(models),
            Err(failure) => {
                let vendor = route.media().name().to_owned();
                let manages = Vault::manages(shard.user(), &route.entry);
                unlisted.push(MediaUnlisted {
                    provider: route.entry.label.clone(),
                    refusal: vendor_refusal(failure.clone(), vendor.clone(), None, manages),
                });
                first_failure.get_or_insert((vendor, failure, manages));
                offered.push(Vec::new());
            }
        }
    }
    (offered, unlisted, first_failure)
}

/// Why no entry makes the kind: there is none, or every one is off.
fn none_made(routes: &[Route]) -> MediaRefusal {
    let off: Vec<String> = routes
        .iter()
        .filter(|route| !route.entry.generates(route.kind))
        .map(|route| route.entry.label.clone())
        .collect();
    if routes.is_empty() || off.len() < routes.len() {
        MediaRefusal::NoProvider
    } else {
        MediaRefusal::AllOff { providers: off }
    }
}

/// The names of `id`'s model of `kind`, `id` first: the model an alias
/// names and every alias of that model, as the catalog states them; `id`
/// alone for a model the catalog does not know. An entry that lists any of
/// them offers the model.
fn model_names(catalog: &Catalog, kind: MediaKind, id: &str) -> Vec<String> {
    let kind = media_catalog::MediaKind::from(kind);
    let Some(known) = catalog.models.iter().find(|model| model.kind == kind && model.id == id) else {
        return vec![id.to_owned()];
    };
    let canonical = known.alias_of.clone().unwrap_or_else(|| known.id.clone());
    let mut names = vec![id.to_owned()];
    let group = catalog.models.iter().filter(|model| {
        model.kind == kind
            && model.vendor == known.vendor
            && (model.id == canonical || model.alias_of.as_deref() == Some(canonical.as_str()))
    });
    for model in group {
        if !names.contains(&model.id) {
            names.push(model.id.clone());
        }
    }
    names
}

/// The model a name of `kind` names: the model itself for an alias.
fn canonical(catalog: &Catalog, kind: MediaKind, id: &str) -> String {
    let kind = media_catalog::MediaKind::from(kind);
    catalog
        .models
        .iter()
        .find(|model| model.kind == kind && model.id == id)
        .and_then(|model| model.alias_of.clone())
        .unwrap_or_else(|| id.to_owned())
}

/// The name of `names` that `listing` offers, the first in their order.
fn listed_name<'a>(listing: &[MediaModelInfo], names: &'a [String]) -> Option<&'a str> {
    names
        .iter()
        .find(|name| listing.iter().any(|model| &model.id == *name))
        .map(String::as_str)
}

/// The models of `kind` of the user's entries, newest first, each with
/// what the catalog states of it and the entries that offer it in the
/// order a call tries them, with why each would or would not make it
/// now; a model no entry that is on offers is not listed.
pub(crate) async fn media_models(shard: &Shard, kind: MediaKind) -> Result<MediaListing, PortFailure> {
    let services = shard.services();
    let routes = routes(services, shard.user(), kind).await.map_err(failed)?;
    let catalog = services.media.current();
    let (offered, unlisted, _) = offerings(shard, &catalog, &routes).await;
    // An alias is a name of a model, not a model: a model is one row, with
    // its aliases among its facts, and every entry that offers any of its
    // names.
    let mut ids: Vec<MediaModelInfo> = Vec::new();
    for listing in &offered {
        for model in listing {
            let id = canonical(&catalog, kind, &model.id);
            if ids.iter().any(|known| known.id == id) {
                continue;
            }
            let listed = listing
                .iter()
                .find(|listed| listed.id == id)
                .cloned()
                .unwrap_or_else(|| if model.id == id { model.clone() } else { MediaModelInfo::id(&id) });
            ids.push(listed);
        }
    }
    let mut models = Vec::new();
    for listed in &ids {
        let names = model_names(&catalog, kind, &listed.id);
        let mut entries = Vec::new();
        let mut taken = false;
        let mut vendors = Vec::new();
        for (route, listing) in routes.iter().zip(&offered) {
            let Some(name) = listed_name(listing, &names) else {
                continue;
            };
            vendors.extend(route.media().vendor());
            let status = match standing(services, route, name).await.map_err(failed)? {
                Standing::Ready if taken => EntryStatus::Next,
                Standing::Ready => {
                    taken = true;
                    EntryStatus::Now
                }
                Standing::UsedUp(until) => EntryStatus::UsedUp { until },
                Standing::Off => EntryStatus::Off,
                Standing::NotSignedIn => EntryStatus::NotSignedIn,
                Standing::TurnedOff => EntryStatus::TurnedOff,
            };
            entries.push(ModelEntry {
                provider: route.entry.label.clone(),
                provider_id: route.entry.id.as_str().to_owned(),
                subscription: route.subscription(),
                account: account_of(shard, route).await,
                confirmed: route.media().confirms_model(),
                status,
            });
        }
        // A model only entries turned off offer is not listed; beside one
        // that an entry makes, those that are off say so
        // (`crates/backend-user-shard/src/provider_settings.rs`).
        let made = entries
            .iter()
            .any(|entry| !matches!(entry.status, EntryStatus::Off | EntryStatus::TurnedOff));
        if made {
            models.push(described(&catalog, kind, listed, &vendors, entries));
        }
    }
    // ISO dates sort as text, newest first and those without one last; a
    // stable sort keeps the listings' order within one date.
    models.sort_by(|a, b| b.released.cmp(&a.released));
    let none = (models.is_empty() && unlisted.is_empty()).then(|| none_made(&routes));
    Ok(MediaListing { models, unlisted, none })
}

/// `listed`, a model of `kind`, with what the catalog states of it, for
/// the first of `vendors` whose catalog entry it is, else what its listing
/// says.
fn described(
    catalog: &Catalog,
    kind: MediaKind,
    listed: &MediaModelInfo,
    vendors: &[MediaVendor],
    entries: Vec<ModelEntry>,
) -> MediaModel {
    let known = vendors.iter().find_map(|vendor| {
        catalog
            .model(vendor.id(), &listed.id)
            .filter(|model| model.kind == kind.into())
            .map(|model| (*vendor, model))
    });
    let Some((vendor, model)) = known else {
        let vendor = vendors.first();
        return MediaModel {
            id: listed.id.clone(),
            vendor: vendor.map(|vendor| vendor.name().to_owned()),
            vendor_id: vendor.map(|vendor| vendor.id().to_owned()),
            name: listed.name.clone(),
            description: listed.description.clone(),
            released: listed.released.clone(),
            alias_of: None,
            aliases: Vec::new(),
            image: None,
            video: None,
            audio: (kind == MediaKind::Audio).then(|| AudioFacts { kinds: listed.kinds.clone(), ..AudioFacts::default() }),
            price: None,
            entries,
        };
    };
    let image = catalog.image(model).map(|image| ImageFacts {
        sizes: image
            .sizes
            .as_ref()
            .map(|sizes| sizes.iter().map(|size| format!("{}x{}", size.width, size.height)).collect()),
        custom_size: image.custom_size.as_ref().map(|custom| CustomSizeRule {
            multiple_of: custom.multiple_of,
            max_edge: custom.max_edge,
            max_ratio: custom.max_ratio,
            min_pixels: custom.min_pixels,
            max_pixels: custom.max_pixels,
        }),
        aspect_ratios: image.aspect_ratios.clone(),
        resolutions: image.resolutions.clone(),
        edits: image.edits,
        max_input_images: image.max_input_images,
        transparent: image.transparent,
    });
    let video = catalog.video(model).map(|video| VideoFacts {
        durations: video.durations.clone(),
        min_duration: video.duration_range.and_then(|range| range.min),
        max_duration: video.duration_range.and_then(|range| range.max),
        aspect_ratios: video.aspect_ratios.clone(),
        resolutions: video.resolutions.clone(),
        first_frame: video.first_frame,
        last_frame: video.last_frame,
        max_reference_images: video.max_reference_images,
        audio: video.audio,
        audio_off: video.audio_off,
        seed: video.seed,
        aspect_with_first_frame: video.aspect_with_first_frame,
        edits: video.edits,
        extends: video.extends,
        max_source_seconds: video.max_source_seconds,
    });
    let audio = (kind == MediaKind::Audio).then(|| {
        let known = catalog.audio(model);
        let kinds = audio_kinds(catalog, Some(vendor), listed);
        let Some(known) = known else {
            return AudioFacts { kinds, ..AudioFacts::default() };
        };
        AudioFacts {
            kinds,
            voices: match &known.voices {
                Some(Voices::Named(names)) => Some(names.clone()),
                _ => None,
            },
            voices_listed: known.voices.as_ref().map(|voices| matches!(voices, Voices::Listed(_))),
            languages: known.languages.clone(),
            instructions: known.instructions,
            max_characters: known.max_characters,
            durations: known.durations.clone(),
            min_duration: known.duration_range.and_then(|range| range.min),
            max_duration: known.duration_range.and_then(|range| range.max),
            lyrics: known.lyrics,
            instrumental: known.instrumental,
            loops: known.loops,
            formats: known.formats.clone(),
        }
    });
    let price = catalog.price(model).map(|price| MediaPrice {
        rates: price
            .rates
            .iter()
            .filter_map(|rate| {
                let modality = match rate.modality {
                    None => None,
                    Some(modality) => Some(modality_name(modality)?.to_owned()),
                };
                Some(PriceRate {
                    unit: unit_name(rate.unit)?.to_owned(),
                    modality,
                    amount: rate.amount,
                    currency: currency_name(rate.currency)?.to_owned(),
                    size: rate.size.clone(),
                    quality: rate.quality.clone(),
                    resolution: rate.resolution.clone(),
                    duration: rate.duration,
                })
            })
            .collect(),
        page: price.page.clone(),
    });
    let aliases = catalog
        .models
        .iter()
        .filter(|alias| alias.vendor == model.vendor && alias.alias_of.as_deref() == Some(model.id.as_str()))
        .map(|alias| alias.id.clone())
        .collect();
    MediaModel {
        id: listed.id.clone(),
        vendor: Some(vendor.name().to_owned()),
        vendor_id: Some(vendor.id().to_owned()),
        name: Some(model.name.clone()),
        description: model
            .description
            .clone()
            .or_else(|| model.alias_of.as_ref().map(|named| format!("Alias of {named}"))),
        released: catalog.released(model).map(|date| date.to_string()).or_else(|| listed.released.clone()),
        alias_of: model.alias_of.clone(),
        aliases,
        image,
        video,
        audio,
        price,
        entries,
    }
}

/// The kinds of audio `listed` makes: what the catalog states of it for
/// `vendor`, else what its listing says, such as a compatible entry's
/// shape's kinds; empty when neither says.
pub(super) fn audio_kinds(catalog: &Catalog, vendor: Option<MediaVendor>, listed: &MediaModelInfo) -> Vec<AudioKind> {
    let made = vendor
        .and_then(|vendor| catalog.model(vendor.id(), &listed.id))
        .and_then(|model| catalog.audio(model))
        .and_then(|audio| audio.makes.as_ref());
    let kinds: Vec<AudioKind> = made
        .into_iter()
        .flatten()
        .filter_map(|output| match output {
            AudioOutput::Speech => Some(AudioKind::Speech),
            AudioOutput::Music => Some(AudioKind::Music),
            AudioOutput::Effect => Some(AudioKind::Effect),
            AudioOutput::Other => None,
        })
        .collect();
    if kinds.is_empty() { listed.kinds.clone() } else { kinds }
}

/// A unit as the listing names it; none for one this backend does not
/// know, whose rate it leaves out.
fn unit_name(unit: PriceUnit) -> Option<&'static str> {
    Some(match unit {
        PriceUnit::Image => "image",
        PriceUnit::Video => "video",
        PriceUnit::Second => "second",
        PriceUnit::Minute => "minute",
        PriceUnit::Song => "song",
        PriceUnit::MillionCharacters => "millionCharacters",
        PriceUnit::MillionInputTokens => "millionInputTokens",
        PriceUnit::MillionCachedInputTokens => "millionCachedInputTokens",
        PriceUnit::MillionOutputTokens => "millionOutputTokens",
        PriceUnit::Other => return None,
    })
}

/// A tokens' kind as the listing names it; none for one this backend does
/// not know, whose rate it leaves out.
fn modality_name(modality: Modality) -> Option<&'static str> {
    Some(match modality {
        Modality::Image => "image",
        Modality::Text => "text",
        Modality::Video => "video",
        Modality::Other => return None,
    })
}

fn currency_name(currency: Currency) -> Option<&'static str> {
    match currency {
        Currency::Usd => Some("USD"),
        Currency::Cny => Some("CNY"),
        Currency::Other => None,
    }
}

/// Makes the image `ask` names for `conversation`, whose ledger rows name
/// the agent and the turn of `caller`, the job that called the command; a
/// call `cancel` stops ends with a cancelled row.
///
/// An image its vendor makes as a job is kept and followed as a video's job
/// is ([`make_video`]), and waited for within the call.
pub(crate) async fn make_image(
    shard: &Shard,
    conversation: &ConversationId,
    caller: Option<&JobCaller>,
    media: &MediaCall,
    ask: ImageAsk,
    cancel: &CancellationToken,
) -> Result<MadeImage, PortFailure> {
    let pick = |made: Made| match made {
        Made::Image { image } => Ok(image),
        _ => Err(refused(MediaRefusal::Failed { message: "the job made no image".to_owned() })),
    };
    if media.resumed {
        resumed_job(shard, conversation, media, cancel).await?;
        return pick(wait_kept(shard, media, cancel).await?);
    }
    let request = ImageRequest {
        model: ask.model.clone(),
        prompt: ask.prompt,
        aspect: ask.aspect.map(|aspect| Aspect {
            width: aspect.width,
            height: aspect.height,
        }),
        transparent: ask.transparent,
        images: ask.images.into_iter().map(input_image).collect(),
    };
    let call = Call {
        shard,
        conversation,
        caller,
        kind: MediaKind::Image,
        model: ask.model,
        provider: ask.provider,
    };
    let asking = ImageCall { request, calling: Calling { media: media.clone(), stop: cancel.clone() } };
    let answered = tokio::select! {
        made = call.make(&asking) => made?,
        () = cancel.cancelled() => return Err(PortError::Ended("the call was stopped".into()).into()),
    };
    match answered {
        Answered::Made(image) => Ok(image),
        Answered::Kept => pick(wait_kept(shard, media, cancel).await?),
    }
}

/// What a call's request answered: what it made, or the job a vendor
/// took, which the shard keeps and follows.
enum Answered<T> {
    Made(T),
    Kept,
}

/// What the job the shard keeps for `media` made, once it ends. A stop of
/// the call ends the wait ([`Shard::watch_call_stop`] says what becomes of
/// the job).
async fn wait_kept(shard: &Shard, media: &MediaCall, cancel: &CancellationToken) -> Result<Made, PortFailure> {
    let of = JobOfCall { call: media.id.clone(), ordinal: media.ordinal };
    tokio::select! {
        made = shard.collected(of) => made,
        () = cancel.cancelled() => Err(PortError::Ended("the call was stopped".into()).into()),
    }
}

/// Asks for the video `ask` names for `conversation`, as [`make_image`] makes an
/// image, and answers the job the vendor took with its steps: each change of its
/// state, then its video.
///
/// A video to change that the user's call made, as its digest finds in the
/// ledger, goes first to that call's entry, which continues its job where the
/// vendor keeps it.
///
/// The job, once the vendor took it, is kept and followed by the shard
/// (`super::media_jobs`): the steps, dropped, stop only the call's waiting.
/// A stop of the call cancels the job where the vendor can, unless the
/// shard is closing, and a call the runner sends again after a restart
/// follows the job its first asked for.
pub(crate) async fn make_video(
    shard: &Shard,
    conversation: &ConversationId,
    caller: Option<&JobCaller>,
    call: &MediaCall,
    ask: VideoAsk,
    cancel: &CancellationToken,
) -> Result<(AskedVideo, VideoJob), PortFailure> {
    let of = JobOfCall { call: call.id.clone(), ordinal: call.ordinal };
    if call.resumed {
        let asked = resumed_video(shard, conversation, call, cancel).await?;
        return Ok((asked, shard.collect_video(of)));
    }
    let request = VideoRequest {
        model: ask.model.clone(),
        prompt: ask.prompt,
        image: ask.image.map(input_image),
        last_frame: ask.last_frame.map(input_image),
        references: ask.references.into_iter().map(input_image).collect(),
        duration: ask.duration,
        aspect: ask.aspect.map(|aspect| Aspect {
            width: aspect.width,
            height: aspect.height,
        }),
        resolution: ask.resolution,
        audio: ask.audio,
        seed: ask.seed,
        source: None,
    };
    let source = match ask.source {
        Some(source) => {
            let bytes = source.bytes.into_bytes();
            let digest = hex::encode(Sha256::digest(&bytes));
            let made = match shard.store().made_video(digest).await {
                Ok(made) => made,
                // The file is sent instead, as for a video no call made.
                Err(error) => {
                    tracing::warn!(event = "media.made_video_unread", error = %error, "the call that made a video to change was not read");
                    None
                }
            };
            let change = match source.change {
                VideoChange::Edit => Change::Edit,
                VideoChange::Extend => Change::Extend,
            };
            Some(SourceFile { change, asked: source.change, bytes, made })
        }
        None => None,
    };
    let making = Call {
        shard,
        conversation,
        caller,
        kind: MediaKind::Video,
        model: ask.model,
        provider: ask.provider,
    };
    let asking = VideoCall { request, source, calling: Calling { media: call.clone(), stop: cancel.clone() } };
    let asked = tokio::select! {
        asked = making.make(&asking) => asked?,
        () = cancel.cancelled() => return Err(PortError::Ended("the call was stopped".into()).into()),
    };
    Ok((asked, shard.collect_video(of)))
}

/// The job of `conversation` a call sent again after a restart asked for,
/// followed, and which the call can come back to once more. A job its call
/// collected already is not asked again: the restart came after the file
/// was handed over.
async fn resumed_job(shard: &Shard, conversation: &ConversationId, media: &MediaCall, cancel: &CancellationToken) -> Result<MediaJob, PortFailure> {
    let of = JobOfCall { call: media.id.clone(), ordinal: media.ordinal };
    let job = match shard.store().media_job(of).await.map_err(failed)? {
        Some(job) if job.usage.conversation == *conversation => job,
        _ => {
            return Err(refused(MediaRefusal::Failed {
                message: "the call came back after its file was handed over, which the server keeps no longer".to_owned(),
            }));
        }
    };
    if job.status == MediaJobStatus::Running {
        shard.follow_media_job(job.clone(), None);
    }
    shard.watch_call_stop(media, cancel.clone());
    media.resumable().await;
    Ok(job)
}

/// The video's job a call sent again after a restart asked for, as its
/// first call was told of it.
async fn resumed_video(shard: &Shard, conversation: &ConversationId, media: &MediaCall, cancel: &CancellationToken) -> Result<AskedVideo, PortFailure> {
    let services = shard.services();
    let job = resumed_job(shard, conversation, media, cancel).await?;
    let provider = match services.vault.entry(job.usage.provider.clone()).await {
        Ok(Some(entry)) => entry.label,
        _ => job.usage.provider.to_string(),
    };
    Ok(AskedVideo {
        job: job.job,
        model: job.usage.model,
        provider,
        vendor: job.follow.vendor,
        account: job.follow.label,
        source: None,
        resumed: true,
    })
}

/// Makes the audio `ask` names for `conversation`, as [`make_image`] makes
/// an image: one request, or a job kept, answered within the call.
pub(crate) async fn make_audio(
    shard: &Shard,
    conversation: &ConversationId,
    caller: Option<&JobCaller>,
    media: &MediaCall,
    ask: AudioAsk,
    cancel: &CancellationToken,
) -> Result<MadeAudio, PortFailure> {
    let pick = |made: Made| match made {
        Made::Audio { audio, .. } => Ok(audio),
        _ => Err(refused(MediaRefusal::Failed { message: "the job made no audio".to_owned() })),
    };
    if media.resumed {
        resumed_job(shard, conversation, media, cancel).await?;
        return pick(wait_kept(shard, media, cancel).await?);
    }
    let request = AudioRequest {
        kind: ask.kind,
        model: ask.model.clone(),
        text: ask.text,
        voice: ask.voice,
        instructions: ask.instructions,
        speed: ask.speed,
        duration: ask.duration,
        instrumental: ask.instrumental,
        seed: ask.seed,
        looped: ask.looped,
        format: match ask.format {
            AudioFileFormat::Mp3 => AudioFormatAsk::Mp3,
            AudioFileFormat::Wav => AudioFormatAsk::Wav,
            AudioFileFormat::Opus => AudioFormatAsk::Opus,
            AudioFileFormat::Flac => AudioFormatAsk::Flac,
        },
    };
    let call = Call {
        shard,
        conversation,
        caller,
        kind: MediaKind::Audio,
        model: ask.model,
        provider: ask.provider,
    };
    let asking = AudioCall { request, calling: Calling { media: media.clone(), stop: cancel.clone() } };
    let answered = tokio::select! {
        made = call.make(&asking) => made?,
        () = cancel.cancelled() => return Err(PortError::Ended("the call was stopped".into()).into()),
    };
    match answered {
        Answered::Made(audio) => Ok(audio),
        Answered::Kept => pick(wait_kept(shard, media, cancel).await?),
    }
}

/// The voices of speech model `model`, as the entry a call of it would
/// take now lists them, read when asked; a listing is no call, and writes
/// no ledger row.
pub(crate) async fn audio_voices(
    shard: &Shard,
    conversation: &ConversationId,
    model: String,
    search: Option<String>,
    cancel: &CancellationToken,
) -> Result<AudioVoices, PortFailure> {
    let call = Call { shard, conversation, caller: None, kind: MediaKind::Audio, model, provider: None };
    let asking = VoicesCall { search };
    tokio::select! {
        voices = call.make(&asking) => voices,
        () = cancel.cancelled() => Err(PortError::Ended("the call was stopped".into()).into()),
    }
}

fn input_image(image: demi_plugin_interface::ImageInput) -> InputImage {
    InputImage {
        media_type: image.kind.media_type(),
        bytes: image.bytes.into_bytes(),
    }
}

/// One call of a kind: which entry makes it, and the account within it.
struct Call<'a> {
    shard: &'a Shard,
    conversation: &'a ConversationId,
    caller: Option<&'a JobCaller>,
    kind: MediaKind,
    /// The model asked for.
    model: String,
    /// The entry `--provider` names.
    provider: Option<String>,
}

impl Call<'_> {
    async fn make<A: Asking>(self, asking: &A) -> Result<A::Made, PortFailure> {
        let shard = self.shard;
        let kind = self.kind;
        let model = self.model.clone();
        let services = shard.services();
        let routes = routes(services, shard.user(), kind).await.map_err(failed)?;
        let agent = self.caller.and_then(|caller| {
            let node = shard.agent().node(&root_of(self.conversation), &caller.node)?;
            Some(node.record().number)
        });
        let catalog = services.media.current();
        let (offered, _, unread) = offerings(shard, &catalog, &routes).await;
        // Each entry that offers any name of the model, with the name it is
        // sent: the one asked when it lists it, else another of the model's.
        let names = model_names(&catalog, kind, &model);
        let sent_names: Vec<(&Route, String)> = routes
            .iter()
            .zip(&offered)
            .filter_map(|(route, listing)| listed_name(listing, &names).map(|name| (route, name.to_owned())))
            .collect();
        let offering: Vec<&Route> = sent_names.iter().map(|(route, _)| *route).collect();
        let sent = |route: &Route| -> String {
            sent_names
                .iter()
                .find(|(named, _)| named.entry.id == route.entry.id)
                .map_or_else(|| model.clone(), |(_, name)| name.clone())
        };
        let mut standings = Vec::new();
        for route in &offering {
            standings.push(standing(services, route, &sent(route)).await.map_err(failed)?);
        }
        let ready = |except: Option<&ProviderEntry>| -> Vec<String> {
            offering
                .iter()
                .zip(&standings)
                .filter(|(route, standing)| **standing == Standing::Ready && Some(&route.entry) != except)
                .map(|(route, _)| route.entry.label.clone())
                .collect()
        };
        // A named entry is the only one tried: the agent asked for it.
        let tried: Vec<&Route> = match &self.provider {
            Some(name) => {
                let entries: Vec<ProviderEntry> = routes.iter().map(|route| route.entry.clone()).collect();
                let entry = match named(&entries, name) {
                    Ok(entry) => entry.clone(),
                    Err(message) => {
                        // An entry that makes none of the kind is named as
                        // one that does not offer the model.
                        let all = services.vault.entries_for(shard.user()).await.map_err(failed)?;
                        let Ok(other) = named(&all, name) else {
                            return Err(failed(message));
                        };
                        return Err(refused(MediaRefusal::Cannot {
                            provider: other.label.clone(),
                            model,
                            why: Unable::NotOffered,
                            can: ready(None),
                        }));
                    }
                };
                let position = offering.iter().position(|route| route.entry.id == entry.id);
                // An entry that shows used up is asked all the same: only its
                // vendor refuses.
                let why = match position {
                    None => Some(Unable::NotOffered),
                    Some(index) => match standings[index] {
                        Standing::UsedUp(_) => None,
                        standing => standing.unable(),
                    },
                };
                if let Some(why) = why {
                    return Err(refused(MediaRefusal::Cannot {
                        can: ready(Some(&entry)),
                        provider: entry.label,
                        model,
                        why,
                    }));
                }
                vec![offering[position.expect("an entry that can is offered")]]
            }
            // The entries ready first, the one the call prefers among them
            // first of all, then those that show used up, the soonest to
            // reset first: only a vendor refuses.
            None => {
                let mut used_up: Vec<(&Route, Option<Timestamp>)> = offering
                    .iter()
                    .zip(&standings)
                    .filter_map(|(route, standing)| match standing {
                        Standing::UsedUp(until) => Some((*route, *until)),
                        _ => None,
                    })
                    .collect();
                used_up.sort_by_key(|(_, until)| (until.is_none(), *until));
                let mut ready: Vec<&Route> = offering
                    .iter()
                    .zip(&standings)
                    .filter(|(_, standing)| **standing == Standing::Ready)
                    .map(|(route, _)| *route)
                    .collect();
                if let Some(preferred) = asking.preferred() {
                    ready.sort_by_key(|route| route.entry.id != *preferred);
                }
                ready.into_iter().chain(used_up.into_iter().map(|(route, _)| route)).collect()
            }
        };
        let mut spent: Option<PortFailure> = None;
        for candidate in tried {
            // The entry as it is now: one turned off, deleted or signed out
            // while an earlier entry's request was under way is not sent to,
            // and its key is the one it has now.
            let Some(route) = current(services, shard.user(), candidate).await.map_err(failed)? else {
                continue;
            };
            let name = sent(&route);
            let listed = routes
                .iter()
                .zip(&offered)
                .find(|(offering, _)| offering.entry.id == route.entry.id)
                .and_then(|(_, listing)| listing.iter().find(|model| model.id == name))
                .cloned()
                .unwrap_or_else(|| MediaModelInfo::id(&name));
            let making = Making {
                shard,
                catalog: &catalog,
                listed,
                manages: Vault::manages(shard.user(), &route.entry),
                turn: self.caller.and_then(|caller| caller.turn.clone()),
                route: &route,
                model: model_of(&name),
                name,
                agent,
                conversation: self.conversation.clone(),
            };
            match making.run(asking).await {
                // Used up: the next entry goes on at once, and the first
                // one's limit is told if none is left.
                Err(failure @ PortFailure::Refused(PortRefusal::Media { refusal: MediaRefusal::UsedUp { .. } })) => {
                    spent.get_or_insert(failure);
                }
                made => return made,
            }
        }
        if let Some(spent) = spent {
            return Err(spent);
        }
        // No entry could: say why, by the first that offers the model.
        if let Some((route, standing)) = offering.iter().zip(&standings).next() {
            let vendor = route.media().name().to_owned();
            return Err(refused(match standing {
                Standing::UsedUp(resets_at) => MediaRefusal::UsedUp { vendor, resets_at: *resets_at, trace: Default::default() },
                Standing::Off => MediaRefusal::Off { model, provider: route.entry.label.clone() },
                Standing::NotSignedIn | Standing::TurnedOff | Standing::Ready => MediaRefusal::Cannot {
                    provider: route.entry.label.clone(),
                    model,
                    why: standing.unable().unwrap_or(Unable::NotOffered),
                    can: Vec::new(),
                },
            }));
        }
        if !routes.iter().any(|route| route.entry.generates(kind)) {
            return Err(refused(none_made(&routes)));
        }
        if let Some((vendor, failure, manages)) = unread {
            return Err(refused(vendor_refusal(failure, vendor, None, manages)));
        }
        Err(refused(MediaRefusal::UnknownModel { model }))
    }
}

/// What a call of one kind sends an entry's maker: an image's or an audio
/// file's one request, a video's ask, or a listing of voices.
trait Asking {
    type Made;

    /// Why the call cannot be sent to `maker` at all, told before anything
    /// is sent; most calls have nothing to check.
    fn check(&self, _making: &Making<'_>, _maker: &MakerOf) -> Result<(), MediaRefusal> {
        Ok(())
    }

    /// The entry the call goes to first among those ready, without
    /// `--provider`; none for the order of Settings.
    fn preferred(&self) -> Option<&ProviderId> {
        None
    }

    /// Sends the call's request as `making.name` to `maker`, on `account`
    /// or with the entry's key, and keeps its ledger row; `label` names
    /// the account as the plugin is told it.
    async fn send(
        &self,
        making: &Making<'_>,
        maker: &MakerOf,
        account: Option<&CredentialId>,
        label: Option<String>,
    ) -> Result<Self::Made, MediaFailure>;
}

/// An image's request, kept as its call's job where the vendor makes one.
struct ImageCall {
    request: ImageRequest,
    calling: Calling,
}

impl Asking for ImageCall {
    type Made = Answered<MadeImage>;

    async fn send(
        &self,
        making: &Making<'_>,
        maker: &MakerOf,
        account: Option<&CredentialId>,
        label: Option<String>,
    ) -> Result<Answered<MadeImage>, MediaFailure> {
        let images = maker.images().expect("an image call's maker makes images");
        let (mut row, span) = making.open_row("image", account);
        let request = ImageRequest { model: making.name.clone(), ..self.request.clone() };
        let answer = MadeImage {
            bytes: Default::default(),
            model: making.name.clone(),
            provider: making.route.entry.label.clone(),
            vendor: images.name().to_owned(),
            account: label.clone(),
            confirmed: images.confirms_model(),
        };
        if let Some(jobs) = images.jobs() {
            let asked_at = making.shard.services().clock.now();
            let asked = jobs.ask(&making.http(), making.catalog, request).instrument(span.clone()).await;
            row.observe(&asked.observed);
            let answer = Made::Image { image: answer };
            keep_job(making, maker, &self.calling, row, span, asked.result, asked_at, label, answer).await?;
            return Ok(Answered::Kept);
        }
        let answer_of = answer;
        let answer = images
            .make(&making.http(), making.catalog, request)
            .instrument(span.clone())
            .await;
        row.observe(&answer.observed);
        let made = answer.result;
        match &made {
            Ok(image) => {
                let row = row.row();
                row.outcome = "answered".to_owned();
                row.usage = image.usage;
                row.vendor_id = image.vendor_id.clone();
            }
            Err(failure) => row.row().outcome = failure.outcome(),
        }
        let (outcome, ms) = (row.row().outcome.clone(), row.ms());
        span.in_scope(|| tracing::info!(event = "media.ended", kind = "image", outcome, ms, "an image call ended"));
        row.write().await;
        let image = made?;
        Ok(Answered::Made(MadeImage { bytes: image.bytes.into(), ..answer_of }))
    }
}

/// An audio file's request, kept as its call's job where the vendor makes
/// one.
struct AudioCall {
    request: AudioRequest,
    calling: Calling,
}

impl Asking for AudioCall {
    type Made = Answered<MadeAudio>;

    /// A model that makes another kind, and speech without a voice for a
    /// vendor that has no default one, fail before anything is sent.
    fn check(&self, making: &Making<'_>, maker: &MakerOf) -> Result<(), MediaRefusal> {
        let audio = maker.audio().expect("an audio call's maker makes audio");
        let kinds = audio_kinds(making.catalog, audio.vendor(), &making.listed);
        if !kinds.is_empty() && !kinds.contains(&self.request.kind) {
            return Err(MediaRefusal::Kind { model: making.name.clone(), asked: self.request.kind, makes: kinds });
        }
        if self.request.kind == AudioKind::Speech && self.request.voice.is_none() && audio.needs_voice() {
            return Err(MediaRefusal::NeedsVoice { vendor: audio.name().to_owned(), model: making.name.clone() });
        }
        Ok(())
    }

    async fn send(
        &self,
        making: &Making<'_>,
        maker: &MakerOf,
        account: Option<&CredentialId>,
        label: Option<String>,
    ) -> Result<Answered<MadeAudio>, MediaFailure> {
        let audio = maker.audio().expect("an audio call's maker makes audio");
        let (mut row, span) = making.open_row("audio", account);
        let asked_speed = self.request.speed;
        let speed = asked_speed.map(|speed| audio::sent_speed(audio, speed));
        let request = AudioRequest { model: making.name.clone(), speed, ..self.request.clone() };
        let shape = audio.shape_name(making.catalog, &request);
        let unsent = audio
            .unsent(making.catalog, &request)
            .into_iter()
            .map(|option| match option {
                audio::AudioOption::Instructions => AudioOption::Instructions,
                audio::AudioOption::Speed => AudioOption::Speed,
                audio::AudioOption::Duration => AudioOption::Duration,
                audio::AudioOption::Instrumental => AudioOption::Instrumental,
                audio::AudioOption::Seed => AudioOption::Seed,
                audio::AudioOption::Loop => AudioOption::Loop,
            })
            .collect();
        let sent_speed = speed.filter(|sent| Some(*sent) != asked_speed);
        let answer_of = MadeAudio {
            bytes: Default::default(),
            pcm: None,
            model: making.name.clone(),
            provider: making.route.entry.label.clone(),
            vendor: audio.name().to_owned(),
            account: label.clone(),
            shape,
            unsent,
            speed: sent_speed,
            cost: None,
        };
        if let Some(jobs) = audio.jobs(making.catalog, request.kind, &request.model) {
            let asked_at = making.shard.services().clock.now();
            let of = request.kind;
            let asked = jobs.ask(&making.http(), making.catalog, request).instrument(span.clone()).await;
            row.observe(&asked.observed);
            let answer = Made::Audio { audio: answer_of, of };
            keep_job(making, maker, &self.calling, row, span, asked.result, asked_at, label, answer).await?;
            return Ok(Answered::Kept);
        }
        let answer = audio.make(&making.http(), making.catalog, request).instrument(span.clone()).await;
        row.observe(&answer.observed);
        let made = answer.result;
        match &made {
            Ok(made) => {
                let row = row.row();
                row.outcome = "answered".to_owned();
                row.usage = made.usage;
                row.vendor_id = made.vendor_id.clone();
                row.cost = made.cost;
                // A file this does not read is the plugin's to refuse; its
                // row keeps no seconds.
                row.media_seconds = match made.pcm {
                    Some(pcm) => Some(pcm.seconds(made.bytes.len())),
                    None => demi_shared_types::audio::read(&made.bytes).ok().map(|facts| facts.seconds),
                };
            }
            Err(failure) => row.row().outcome = failure.outcome(),
        }
        let (outcome, ms) = (row.row().outcome.clone(), row.ms());
        span.in_scope(|| tracing::info!(event = "media.ended", kind = "audio", outcome, ms, "an audio call ended"));
        row.write().await;
        let made = made?;
        Ok(Answered::Made(MadeAudio {
            bytes: made.bytes.into(),
            pcm: made.pcm.map(|pcm| PcmShape { sample_rate: pcm.sample_rate(), channels: pcm.channels(), bits: pcm.bits() }),
            cost: made.cost,
            ..answer_of
        }))
    }
}

/// The call a request belongs to, and its stop.
#[derive(Clone)]
struct Calling {
    media: MediaCall,
    stop: CancellationToken,
}

/// Keeps the job a vendor took for `calling`, the id `asked` names, and
/// follows it: its row in the control database, written before the call is
/// marked resumable, and its open usage `row`, which the follower writes at
/// the job's end. An ask the vendor refused writes the row now.
///
/// The keeping is the shard's task, which the call's stop does not cut
/// short: a stop that comes meanwhile leaves the job to nobody, as one that
/// comes later does ([`Shard::watch_call_stop`]).
async fn keep_job(
    making: &Making<'_>,
    maker: &MakerOf,
    calling: &Calling,
    mut row: OpenRow,
    span: tracing::Span,
    asked: Result<String, MediaFailure>,
    asked_at: Timestamp,
    label: Option<String>,
    answer: Made,
) -> Result<String, MediaFailure> {
    let kind = answer.kind();
    let job = match asked {
        Ok(job) => job,
        Err(failure) => {
            row.row().outcome = failure.outcome();
            let (outcome, ms) = (row.row().outcome.clone(), row.ms());
            span.in_scope(|| tracing::info!(event = "media.ended", kind = kind.plural(), stage = "ask", outcome, ms, "a media call ended at its ask"));
            row.write().await;
            return Err(failure);
        }
    };
    span.in_scope(|| tracing::info!(event = "media_job.asked", kind = kind.plural(), job = %job, "a media job was asked for"));
    row.row().vendor_id = Some(job.clone());
    let media = &calling.media;
    let kept = MediaJob {
        of: JobOfCall { call: media.id.clone(), ordinal: media.ordinal },
        kind,
        job: job.clone(),
        asked_at,
        usage: row.keep(),
        follow: Follow { vendor: maker.media().name().to_owned(), label, manages: making.manages, answer },
        status: MediaJobStatus::Running,
    };
    let shard = making.shard.this();
    let reader = Reader { maker: maker.clone(), http: making.http() };
    let calling = calling.clone();
    let (done, kept_now) = tokio::sync::oneshot::channel();
    making.shard.tasks().spawn_local(
        async move {
            // A job no row keeps is followed all the same, so its cost is
            // recorded; its call cannot collect it.
            match shard.store().insert_media_job(kept.clone()).await {
                Ok(true) => {}
                Ok(false) => tracing::info!(event = "media_job.unkept", cause = "conversation_gone", "a media job was not kept"),
                Err(error) => tracing::warn!(event = "media_job.unkept", %error, "a media job was not kept"),
            }
            shard.follow_media_job(kept, Some(reader));
            shard.watch_call_stop(&calling.media, calling.stop.clone());
            calling.media.resumable().await;
            // A call that stopped meanwhile waits for nothing.
            let _ = done.send(());
        }
        .instrument(span),
    );
    let _ = kept_now.await;
    Ok(job)
}

/// A listing of a speech model's voices: no request that is paid for, so
/// it writes no ledger row.
struct VoicesCall {
    search: Option<String>,
}

impl Asking for VoicesCall {
    type Made = AudioVoices;

    async fn send(
        &self,
        making: &Making<'_>,
        maker: &MakerOf,
        _: Option<&CredentialId>,
        _: Option<String>,
    ) -> Result<AudioVoices, MediaFailure> {
        let audio = maker.audio().expect("a listing of voices is an audio maker's");
        let voices = audio
            .voices(&making.http(), making.catalog, &making.name, self.search.as_deref())
            .await?;
        Ok(AudioVoices {
            model: making.name.clone(),
            provider: making.route.entry.label.clone(),
            vendor: audio.name().to_owned(),
            voices: voices.map(|voices| {
                voices
                    .into_iter()
                    .map(|voice| VoiceInfo {
                        id: voice.id,
                        name: voice.name,
                        languages: voice.languages,
                        description: voice.description,
                        own: voice.own,
                    })
                    .collect()
            }),
        })
    }
}

/// A video's ask, which starts its job, kept for its call.
struct VideoCall {
    request: VideoRequest,
    source: Option<SourceFile>,
    calling: Calling,
}

/// The video an edit or an extension changes, and the call that made it
/// when one of the user's did.
struct SourceFile {
    change: Change,
    asked: VideoChange,
    bytes: Bytes,
    made: Option<MadeVideoCall>,
}

impl Asking for VideoCall {
    type Made = AskedVideo;

    /// A change the entry's request has no field for, and an option the
    /// model's catalog entry says it does not take, fail before anything is
    /// sent.
    fn check(&self, making: &Making<'_>, maker: &MakerOf) -> Result<(), MediaRefusal> {
        let videos = maker.videos().expect("a video call's maker makes videos");
        if let Some(source) = &self.source
            && !videos.changes().contains(&source.change)
        {
            return Err(MediaRefusal::NoChange { model: making.name.clone(), change: source.asked });
        }
        match video_takes(making, videos).and_then(|takes| videos::untaken(takes, &self.request)) {
            Some(takes) => Err(MediaRefusal::Untaken { model: making.name.clone(), takes }),
            None => Ok(()),
        }
    }

    /// The entry of the call that made the video to change.
    fn preferred(&self) -> Option<&ProviderId> {
        self.source.as_ref()?.made.as_ref().map(|made| &made.provider)
    }

    /// Asks for the video, then keeps the job the vendor took and follows
    /// it (`super::media_jobs`), which writes the call's row at its end.
    async fn send(
        &self,
        making: &Making<'_>,
        maker: &MakerOf,
        account: Option<&CredentialId>,
        label: Option<String>,
    ) -> Result<AskedVideo, MediaFailure> {
        let videos = maker.videos().expect("a video call's maker makes videos");
        let services = making.shard.services();
        let (mut row, span) = making.open_row("video", account);
        let mut request = VideoRequest { model: making.name.clone(), ..self.request.clone() };
        // A new video asked no length is the shortest the model takes, as
        // a vendor's own default is often its longest; an edit keeps its
        // source's length.
        if self.source.is_none() && request.duration.is_none() {
            request.duration = video_takes(making, videos).and_then(VideoCapabilities::shortest);
        }
        // A video the entry's own job made, as the model it is asked of,
        // continues that job where the vendor keeps it; any other is sent.
        let kept = self.source.as_ref().and_then(|source| source.made.as_ref()).filter(|made| {
            videos.continues_jobs() && made.provider == making.route.entry.id && made.model == making.name
        });
        let mut sent = self.source.as_ref().map(|source| {
            let (video, sent) = match kept {
                Some(made) => (SourceVideo::Job(made.job.clone()), SentSource::Job { job: made.job.clone() }),
                None => (SourceVideo::File(source.bytes.clone()), SentSource::File),
            };
            request.source = Some(VideoSource { change: source.change, video });
            sent
        });
        let http = making.http();
        // The job's bound counts from its ask, the ask's own wait
        // included.
        let asked_at = services.clock.now();
        let mut answer = videos.ask(&http, making.catalog, request.clone()).instrument(span.clone()).await;
        row.observe(&answer.observed);
        // A job the vendor no longer keeps: the file goes instead.
        if let (Some(source), Some(SentSource::Job { job })) = (&self.source, &sent)
            && matches!(answer.result, Err(MediaFailure::SourceGone { .. }))
        {
            span.in_scope(|| tracing::info!(event = "media_job.source_gone", job = %job, "the vendor no longer keeps the job of a video to change"));
            sent = Some(SentSource::FileInstead { job: job.clone() });
            request.source = Some(VideoSource { change: source.change, video: SourceVideo::File(source.bytes.clone()) });
            answer = videos.ask(&http, making.catalog, request.clone()).instrument(span.clone()).await;
            row.observe(&answer.observed);
        }
        let vendor = videos.name().to_owned();
        let made = Made::Video { video: MadeVideo { bytes: Default::default(), cost: None } };
        let job = keep_job(making, maker, &self.calling, row, span, answer.result, asked_at, label.clone(), made).await?;
        Ok(AskedVideo {
            job,
            model: making.name.clone(),
            provider: making.route.entry.label.clone(),
            vendor,
            account: label,
            source: sent,
            resumed: false,
        })
    }
}

/// What the model a video call makes takes, as the call's catalog states it
/// of the maker's vendor; none for a model it does not know.
fn video_takes<'a>(making: &'a Making<'_>, videos: &dyn VideoMaker) -> Option<&'a VideoCapabilities> {
    let vendor = videos.vendor()?;
    let known = making.catalog.model(vendor.id(), &making.name)?;
    making.catalog.video(known)
}

/// One call made on an entry: the account it takes under the entry's
/// rotation, and another at once when that one is used up; a key's entry
/// has no account.
struct Making<'a> {
    shard: &'a Shard,
    /// What the model takes, as the call's catalog says.
    catalog: &'a Arc<Catalog>,
    /// The model as the entry's listing names it.
    listed: MediaModelInfo,
    route: &'a Route,
    /// The name the model is sent as.
    name: String,
    /// The calling agent's number, which the ledger rows name.
    agent: Option<u64>,
    /// The turn whose shell call started the calling job, which the
    /// ledger rows name even after it ended.
    turn: Option<TurnId>,
    /// Whether the user manages the entry, and so may read its accounts.
    manages: bool,
    conversation: ConversationId,
    /// The model as a rotation reads a window's scope.
    model: Model,
}

impl Making<'_> {
    /// The client the call's requests go through: the shard's, reaching
    /// the entry owner's devices.
    fn http(&self) -> demi_provider_common::ProviderHttp {
        self.route.http(self.shard.services(), self.shard.http())
    }

    async fn run<A: Asking>(self, asking: &A) -> Result<A::Made, PortFailure> {
        let vendor = self.route.media().name().to_owned();
        let made = on_accounts(self.shard, self.route, &self.model, self.manages, async |maker, account, label| {
            asking.check(&self, maker).map_err(Unmade::Refusal)?;
            asking
                .send(&self, maker, account, label.clone())
                .await
                .map_err(|failure| Unmade::Vendor { failure, label })
        })
        .await;
        made.map_err(|unmade| {
            refused(match unmade {
                Unmade::Refusal(refusal) => refusal,
                Unmade::Vendor { failure, label } => vendor_refusal(failure, vendor, label, self.manages),
            })
        })
    }

    /// The ledger row of a request sent now on `account`, or with the
    /// entry's key, open until the call ends, and its span.
    fn open_row(&self, purpose: &'static str, account: Option<&CredentialId>) -> (OpenRow, tracing::Span) {
        let services = self.shard.services();
        let entry = &self.route.entry;
        let request_id = uuid::Uuid::new_v4().to_string();
        let span = tracing::info_span!(
            "media_call",
            purpose,
            provider_request_id = %request_id,
            conversation = %self.conversation,
            user = %self.shard.user(),
            turn = self.turn.as_ref().map(tracing::field::display),
            node = self.agent,
            account = account.map(tracing::field::display),
            provider = %entry.id,
            model = %self.name,
        );
        let now = services.clock.now();
        let row = UsageRow {
            conversation: self.conversation.clone(),
            agent: self.agent,
            turn: self.turn.as_ref().map(ToString::to_string),
            purpose,
            provider: entry.id.clone(),
            account: account.cloned(),
            model: self.name.clone(),
            request: request_id,
            vendor_id: None,
            usage: None,
            cache_write_1h_tokens: 0,
            outcome: "cancelled".to_owned(),
            quota: None,
            started_at: now,
            ended_at: now,
            media_seconds: None,
            media_digest: None,
            cost: None,
            // A medium's tokens have no catalog price: the vendor's own
            // cost prices the call (`crates/core-store/src/usage.rs`),
            // and it asked for no prompt of a session.
            price: None,
            system_hash: None,
            tools_hash: None,
        };
        (OpenRow { row: Some(row), shard: self.shard.this() }, span)
    }
}

/// Why a call on an entry ended without what it made.
pub(super) enum Unmade {
    /// Demi's, told before a request is sent or instead of the vendor's
    /// answer, such as every account used up.
    Refusal(MediaRefusal),
    /// The vendor's, on the account `label` names to whoever manages the
    /// entry; none for a key or for anyone else.
    Vendor { failure: MediaFailure, label: Option<String> },
}

/// Sends a call on `route`'s entry with `send`: once with the entry's key, or
/// on the account the entry's rotation takes, and on another at once when the
/// vendor finds that one used up. Each used-up account is marked as a turn's
/// failure marks it. `send` is told the account and, when `manages`, its
/// label.
pub(super) async fn on_accounts<T>(
    shard: &Shard,
    route: &Route,
    model: &Model,
    manages: bool,
    mut send: impl AsyncFnMut(&MakerOf, Option<&CredentialId>, Option<String>) -> Result<T, Unmade>,
) -> Result<T, Unmade> {
    if !route.subscription() {
        return send(&route.maker, None, None).await;
    }
    let entry = &route.entry;
    let vendor = route.media().name().to_owned();
    let failed = |error: &dyn std::fmt::Display| Unmade::Refusal(MediaRefusal::Failed { message: error.to_string() });
    let services = shard.services();
    let provider_registry = &services.provider_registry;
    let rotation = entry.rotation();
    // The accounts that refused the call, which it does not ask again.
    let mut tried: Vec<CredentialId> = Vec::new();
    let mut last_moment = None;
    // What the vendor named to trace the last usage limit, which the
    // call's words end with.
    let mut last_trace = VendorTrace::default();
    loop {
        let used_up = |resets_at, trace: &VendorTrace| {
            Unmade::Refusal(MediaRefusal::UsedUp { vendor: vendor.clone(), resets_at, trace: trace.clone() })
        };
        // Another account is sent to only while the entry may still
        // make its kind.
        if !tried.is_empty() && current(services, shard.user(), route).await.map_err(|error| failed(&error))?.is_none() {
            return Err(used_up(last_moment, &last_trace));
        }
        let (account, running) = match rotation {
            Rotation::Manual => {
                let active = entry.active().cloned();
                (active.ok_or(Unmade::Refusal(MediaRefusal::NoProvider))?, None)
            }
            Rotation::InOrder | Rotation::Smart => {
                let (choice, _) =
                    provider_registry.take_account(entry, model, &tried).await.map_err(|error| failed(&error))?;
                match choice {
                    Some(Choice::Account(running)) => (running.account().clone(), Some(running)),
                    Some(Choice::AllTried) => return Err(used_up(last_moment, &last_trace)),
                    None => unreachable!("a rotating entry takes an account or finds them used up"),
                }
            }
        };
        let provider = provider_registry
            .for_account(entry, &account)
            .await
            .map_err(|error| failed(&error))?
            .ok_or(Unmade::Refusal(MediaRefusal::NoProvider))?;
        // The account is named only to whoever manages the entry: on a
        // shared instance it is the administrator's
        // (`crates/core-providers/src/vault/entries.rs`).
        let label = if manages {
            let record = services.vault.account(entry.id.clone(), account.clone()).await.ok().flatten();
            Some(record.map_or_else(|| account.to_string(), |record| record.label))
        } else {
            None
        };
        let Some(maker) = provider_registry.maker(entry, &provider, route.kind) else {
            return Err(Unmade::Refusal(MediaRefusal::NoProvider));
        };
        let sent = send(&maker, Some(&account), label).await;
        // The call's account counts no longer: its request ended, and a
        // video's job, once asked for, is the vendor's.
        drop(running);
        let (retry_at, trace) = match sent {
            Err(Unmade::Vendor { failure: MediaFailure::UsageLimit { retry_at, trace }, .. }) => (retry_at, trace),
            sent => return sent,
        };
        let now = services.clock.now();
        let moment = retry_at
            .or_else(|| provider.quota()?.used_up_until(now))
            .filter(|moment| *moment > now);
        let source = if moment.is_none() {
            "default"
        } else if retry_at.is_some() {
            "vendor_refusal"
        } else {
            "snapshot"
        };
        let until = moment.unwrap_or_else(|| {
            Timestamp::from_millisecond(now.as_millisecond() + USED_UP_FOR_MS)
                .expect("half an hour from now is a time")
        });
        // Used up as a turn's failure marks it, whatever the call does
        // next.
        provider_registry.mark_used_up(&entry.id, &account, until, source);
        if rotation == Rotation::Manual {
            return Err(used_up(moment, &trace));
        }
        tried.push(account);
        last_moment = moment;
        last_trace = trace;
    }
}

/// What the plugin is told of a call the vendor did not answer as asked;
/// `manages` says whether the user manages the entry.
pub(crate) fn vendor_refusal(failure: MediaFailure, vendor: String, account: Option<String>, manages: bool) -> MediaRefusal {
    match failure {
        MediaFailure::SignIn(failure) if matches!(failure.reason, AuthReason::Store(_)) => {
            MediaRefusal::Failed { message: failure.message() }
        }
        MediaFailure::SignIn(_) => MediaRefusal::Expired { vendor, account },
        MediaFailure::KeyRefused { trace, .. } => MediaRefusal::KeyRefused { vendor, manages, trace },
        MediaFailure::Refused { reason, trace, .. } => MediaRefusal::Refused { vendor, reason, trace },
        MediaFailure::Shape { takes, trace, .. } => MediaRefusal::Shape { vendor, takes, trace },
        MediaFailure::Unanswered { code: Some(ErrorCode::RateLimit), trace, .. } => MediaRefusal::Limited { vendor, trace },
        MediaFailure::Unanswered { cause, trace, .. } => MediaRefusal::Unanswered { vendor, cause, trace },
        MediaFailure::UsageLimit { retry_at, trace } => MediaRefusal::UsedUp { vendor, resets_at: retry_at, trace },
        MediaFailure::SourceGone { trace } => MediaRefusal::Refused {
            vendor,
            reason: Some("it no longer keeps the job of the video to change".to_owned()),
            trace,
        },
    }
}

/// The ledger row of a call open now, written as cancelled if the call is
/// dropped before it ends.
struct OpenRow {
    row: Option<UsageRow>,
    shard: Rc<Shard>,
}

impl OpenRow {
    fn row(&mut self) -> &mut UsageRow {
        self.row.as_mut().expect("a call's row is open until it is written")
    }

    /// How long the call has lasted, in milliseconds.
    fn ms(&mut self) -> u64 {
        let now = self.shard.services().clock.now().as_millisecond();
        let started = self.row().started_at.as_millisecond();
        u64::try_from(now - started).unwrap_or(0)
    }

    /// Keeps the windows of the account's quota a response reported.
    fn observe(&mut self, observed: &[QuotaWindow]) {
        if observed.is_empty() {
            return;
        }
        merge_windows(self.row().quota.get_or_insert_with(Vec::new), observed);
    }

    /// The row, left open for whoever ends the call: a job a vendor took
    /// writes its row at the job's end.
    fn keep(mut self) -> UsageRow {
        self.row.take().expect("a call's row is kept once")
    }

    /// Writes the row, ended now.
    async fn write(&mut self) {
        let mut row = self.row.take().expect("a call's row is written once");
        row.ended_at = self.shard.services().clock.now();
        let lost = (row.conversation.clone(), row.provider.clone(), row.model.clone());
        if let Err(error) = self.shard.store().append_usage(row).await {
            // The row is lost; what the call made still comes back
            // (`crates/core-store/src/usage.rs`).
            tracing::warn!(
                event = "usage_ledger.write_failed",
                conversation = %lost.0,
                provider = %lost.1,
                model = lost.2.as_str(),
                error = &error as &dyn std::error::Error,
                "a usage ledger row was not written"
            );
        }
    }
}

impl Drop for OpenRow {
    fn drop(&mut self) {
        if let Some(mut row) = self.row.take() {
            row.ended_at = self.shard.services().clock.now();
            let store = self.shard.store().clone();
            let lost = (row.conversation.clone(), row.provider.clone(), row.model.clone());
            // Nothing awaits a dropped call any more.
            tokio::spawn(async move {
                if let Err(error) = store.append_usage(row).await {
                    tracing::warn!(
                        event = "usage_ledger.write_failed",
                        conversation = %lost.0,
                        provider = %lost.1,
                        model = lost.2.as_str(),
                        error = &error as &dyn std::error::Error,
                        "a usage ledger row was not written"
                    );
                }
            });
        }
    }
}
