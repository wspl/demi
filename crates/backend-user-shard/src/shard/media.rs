//! The Providers service of the plugins' port for generated media
//! (`media.md`, `plugins.md` § The contract): the models of one kind the
//! user's entries offer, each with every fact the catalog of media models
//! states and the entries that offer it; one image made through an entry's
//! account or key; and one video's job, from its ask to its download.
//!
//! Which entry makes a call (`media.md` § Which entry makes a call): the
//! one `--provider` names, which fails the call when it cannot; otherwise
//! the first that can make the model now, the subscriptions before the
//! keys, each in the order of Settings › Providers. An entry can when the
//! user has it on, its switch for the kind is on, it is signed in or has
//! its key, it offers the model and an account of it is not used up. An
//! entry whose last account is used up by the call's request hands it on
//! to the next; a refusal of the request is never handed on, and neither is
//! a video's job once the vendor took it, since it is paid for
//! (`videos.md` § Accounts and failures). Within an entry the call takes
//! the account under the entry's rotation, as a new agent takes one at its
//! first request, and another at once when that one is used up. The
//! provider sends the request, renewing its account's sign-in, and each
//! call is a usage ledger row with the purpose of its kind. What the agent
//! reads of it, the plugin words.

use std::rc::Rc;
use std::time::Duration;

use demi_backend_database::usage::{UsageRow, merge_windows};
use demi_backend_plugins::VideoJob;
use demi_backend_providers::llm::assembly::{AssemblyError, MakerOf};
use demi_backend_providers::llm::rotation::{Choice, USED_UP_FOR_MS};
use demi_backend_providers::vault::entries::{EntryCredential, ProviderEntry};
use demi_backend_host_access::root_of;
use demi_host_interface::{JobCaller, PortError};
use demi_plugin_interface::{
    AskedVideo, CustomSizeRule, EntryStatus, ImageAsk, ImageFacts, MadeImage, MadeVideo, MediaListing, MediaModel,
    MediaPrice, MediaRefusal, MediaUnlisted, ModelEntry, PortFailure, PortRefusal, PriceRate, Unable, VideoAsk,
    VideoFacts, VideoResolution, VideoStep,
};
use demi_provider_common::credentials::AuthReason;
use demi_provider_common::images::{Aspect, ImageRequest, InputImage};
use demi_provider_common::media::{MediaFailure, MediaModelInfo, MediaVendor};
use demi_provider_common::media_catalog::{self, Catalog, Currency, Modality, PriceUnit};
use demi_provider_common::videos::{self, Done, JobState, Resolution, VideoOption, VideoRequest};
use demi_shared_types::{MediaKind, Model, QuotaWindow, Rotation, Timestamp, TurnId, mp4};
use demi_web_api_protocol::ids::{ConversationId, CredentialId};
use demi_web_api_protocol::providers::EntryMediaModel;
use futures_util::StreamExt as _;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::Shard;
use super::provider_commands::named;
use crate::services::Services;

/// An entry that makes media of a kind, with what makes them for its
/// active account, in the order a call tries the entries.
struct Route {
    entry: ProviderEntry,
    kind: MediaKind,
    maker: MakerOf,
    /// The user turned the entry off in Settings › Providers.
    hidden: bool,
}

impl Route {
    fn media(&self) -> &dyn demi_provider_common::media::MediaMaker {
        self.maker.media()
    }

    /// The client the entry's calls go through on `client`, the shard's or
    /// the shared services'.
    fn http(&self, services: &Services, client: &reqwest::Client) -> demi_provider_common::ProviderHttp {
        services.assembly.http_for(&self.entry, client.clone())
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
) -> Result<Vec<Route>, AssemblyError> {
    let preferences = services.control.preferences(user.clone()).await?;
    let owner = services.vault.owner_for(user).await?;
    let mut routes = Vec::new();
    for entry in services.vault.entries(owner).await? {
        if !services.assembly.media_shapes(&entry).makes(kind) {
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

async fn route(
    services: &Services,
    entry: ProviderEntry,
    preferences: &demi_web_api_protocol::settings::Preferences,
    kind: MediaKind,
) -> Result<Option<Route>, AssemblyError> {
    let provider = match services.assembly.provider_for(&entry).await {
        Ok(provider) => provider,
        // An entry that lacks what it needs makes nothing yet.
        Err(AssemblyError::Incomplete) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(maker) = services.assembly.maker(&entry, &provider, kind) else {
        return Ok(None);
    };
    Ok(Some(Route {
        hidden: preferences.hides_provider(&entry.id),
        entry,
        kind,
        maker,
    }))
}

/// Entry `route` as it is now, while it may still make its kind: it
/// exists, the user has it on, its switch for the kind is on, and it is
/// signed in or has its key, read again right before a request is sent,
/// since any of them may have changed while an earlier entry's request was
/// under way.
async fn current(
    services: &Services,
    user: &demi_web_api_protocol::ids::UserId,
    route: &Route,
) -> Result<Option<Route>, AssemblyError> {
    let Some(entry) = services.vault.entry(route.entry.id.clone()).await? else {
        return Ok(None);
    };
    let preferences = services.control.preferences(user.clone()).await?;
    let Some(now) = self::route(services, entry, &preferences, route.kind).await? else {
        return Ok(None);
    };
    let usable = !now.hidden
        && now.entry.generates(route.kind)
        && !matches!(&now.entry.credential, EntryCredential::Subscription { active: None, .. });
    Ok(usable.then_some(now))
}

/// The model `id` as a rotation reads a window's scope.
fn model_of(id: &str) -> Model {
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
async fn standing(services: &Services, route: &Route, model: &str) -> Result<Standing, AssemblyError> {
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
    let assembly = &services.assembly;
    let mut candidates = assembly.candidates(entry, false).await?;
    if entry.rotation() == Rotation::Manual {
        candidates.retain(|candidate| &candidate.id == active);
    }
    // An entry whose accounts its vendor all signed out has none to use.
    if candidates.is_empty() {
        return Ok(Standing::NotSignedIn);
    }
    let now = assembly.now();
    // A mark a call left counts as a window does, with its moment.
    Ok(match assembly.rotation().all_used_up(&entry.id, &candidates, &model_of(model), now) {
        Some(until) => Standing::UsedUp(until),
        None => Standing::Ready,
    })
}

/// Each entry's models of `kind`, by entry id, newest first, for the
/// settings page's Images and Videos sections (`providers.md` § Provider
/// management): whether or not its switch for the kind is on, and an alias
/// left out beside the model it names. An entry whose models could not be
/// listed has none here; the agent's listing says why.
pub async fn entry_media_models(
    services: &Services,
    user: &demi_web_api_protocol::ids::UserId,
    kind: MediaKind,
) -> Result<std::collections::HashMap<String, Vec<EntryMediaModel>>, AssemblyError> {
    let routes = routes(services, user, kind).await?;
    let catalog = services.media.current();
    let https: Vec<_> = routes.iter().map(|route| route.http(services, services.assembly.http())).collect();
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
                }
            })
            .collect();
        models.sort_by(|a, b| b.released.cmp(&a.released));
        entries.insert(route.entry.id.as_str().to_owned(), models);
    }
    Ok(entries)
}

/// Whether the user manages the entries they make media with, and so may
/// read their accounts and check their keys.
async fn manages(shard: &Shard) -> Result<bool, PortFailure> {
    let user = shard.user();
    Ok(shard.services().vault.owner_for(user).await.map_err(failed)? == *user)
}

/// The active account's label of `route`'s entry, for a user who manages
/// it.
async fn account_of(shard: &Shard, route: &Route, manages: bool) -> Option<String> {
    let active = route.entry.active().filter(|_| manages)?;
    let record = shard.services().vault.account(route.entry.id.clone(), active.clone()).await.ok().flatten();
    Some(record.map_or_else(|| active.to_string(), |record| record.label))
}

/// Each route's models, in the routes' order; a route whose models could
/// not be listed offers none and is told in `unlisted`.
async fn offerings(
    shard: &Shard,
    catalog: &Catalog,
    routes: &[Route],
    manages: bool,
) -> (Vec<Vec<MediaModelInfo>>, Vec<MediaUnlisted>, Option<(String, MediaFailure)>) {
    let mut offered = Vec::new();
    let mut unlisted = Vec::new();
    let mut first_failure = None;
    for route in routes {
        match route.media().models(&route.http(shard.services(), shard.http()), catalog).await {
            Ok(models) => offered.push(models),
            Err(failure) => {
                let vendor = route.media().name().to_owned();
                unlisted.push(MediaUnlisted {
                    provider: route.entry.label.clone(),
                    refusal: vendor_refusal(failure.clone(), vendor.clone(), None, manages),
                });
                first_failure.get_or_insert((vendor, failure));
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
/// them offers the model (`media.md` § What the agent is told).
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
/// order a call tries them, with why each would or would not make it now
/// (`media.md` § What the agent is told); a model no entry that is on
/// offers is not listed.
pub(crate) async fn media_models(shard: &Shard, kind: MediaKind) -> Result<MediaListing, PortFailure> {
    let services = shard.services();
    let routes = routes(services, shard.user(), kind).await.map_err(failed)?;
    let manages = manages(shard).await?;
    let catalog = services.media.current();
    let (offered, unlisted, _) = offerings(shard, &catalog, &routes, manages).await;
    // An alias is a name of a model, not a model: a model is one row, with
    // its aliases among its facts, and every entry that offers any of its
    // names (`media.md` § What the agent is told).
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
                account: account_of(shard, route, manages).await,
                confirmed: route.media().confirms_model(),
                status,
            });
        }
        // A model only entries turned off offer is not listed; beside one
        // that an entry makes, those that are off say so (`providers.md`
        // § Provider management, `media.md` § What the agent is told).
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
        price,
        entries,
    }
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
pub(crate) async fn make_image(
    shard: &Shard,
    conversation: &ConversationId,
    caller: Option<&JobCaller>,
    ask: ImageAsk,
    cancel: &CancellationToken,
) -> Result<MadeImage, PortFailure> {
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
    let asking = ImageCall { request };
    tokio::select! {
        made = call.make(&asking) => made,
        () = cancel.cancelled() => Err(PortError::Ended("the call was stopped".into()).into()),
    }
}

/// Asks for the video `ask` names for `conversation`, as [`make_image`]
/// makes an image, and answers the job the vendor took with its steps:
/// each change of its state, then its video. The steps, dropped before the
/// video, stop the waiting, cancel the job where the vendor can and write
/// the call's cancelled row (`videos.md` § A call is a job).
pub(crate) async fn make_video(
    shard: &Shard,
    conversation: &ConversationId,
    caller: Option<&JobCaller>,
    ask: VideoAsk,
    cancel: &CancellationToken,
) -> Result<(AskedVideo, VideoJob), PortFailure> {
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
        resolution: ask.resolution.map(|resolution| match resolution {
            VideoResolution::P480 => Resolution::P480,
            VideoResolution::P720 => Resolution::P720,
            VideoResolution::P1080 => Resolution::P1080,
            VideoResolution::K4 => Resolution::K4,
        }),
        audio: ask.audio,
        seed: ask.seed,
    };
    let call = Call {
        shard,
        conversation,
        caller,
        kind: MediaKind::Video,
        model: ask.model,
        provider: ask.provider,
    };
    let asking = VideoCall { request };
    tokio::select! {
        asked = call.make(&asking) => asked,
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
        let manages = manages(shard).await?;
        let catalog = services.media.current();
        let (offered, _, unread) = offerings(shard, &catalog, &routes, manages).await;
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
                        let owner = services.vault.owner_for(shard.user()).await.map_err(failed)?;
                        let all = services.vault.entries(owner).await.map_err(failed)?;
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
                let why = match position {
                    None => Some(Unable::NotOffered),
                    Some(index) => standings[index].unable(),
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
            None => offering
                .iter()
                .zip(&standings)
                .filter(|(_, standing)| **standing == Standing::Ready)
                .map(|(route, _)| *route)
                .collect(),
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
            let making = Making {
                shard,
                catalog: &catalog,
                manages,
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
                Standing::UsedUp(resets_at) => MediaRefusal::UsedUp { vendor, resets_at: *resets_at },
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
        if let Some((vendor, failure)) = unread {
            return Err(refused(vendor_refusal(failure, vendor, None, manages)));
        }
        Err(refused(MediaRefusal::UnknownModel { model }))
    }
}

/// What a call of one kind sends an entry's maker: an image's one request,
/// or a video's ask.
trait Asking {
    type Made;

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

/// An image's one request.
struct ImageCall {
    request: ImageRequest,
}

impl Asking for ImageCall {
    type Made = MadeImage;

    async fn send(
        &self,
        making: &Making<'_>,
        maker: &MakerOf,
        account: Option<&CredentialId>,
        label: Option<String>,
    ) -> Result<MadeImage, MediaFailure> {
        let images = maker.images().expect("an image call's maker makes images");
        let (mut row, span) = making.open_row("image", account);
        let request = ImageRequest { model: making.name.clone(), ..self.request.clone() };
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
        span.in_scope(|| tracing::info!(outcome = %row.row().outcome, "an image call ended"));
        row.write().await;
        let image = made?;
        Ok(MadeImage {
            bytes: image.bytes.into(),
            model: making.name.clone(),
            provider: making.route.entry.label.clone(),
            vendor: images.name().to_owned(),
            account: label,
            confirmed: images.confirms_model(),
        })
    }
}

/// A video's ask, which starts its job.
struct VideoCall {
    request: VideoRequest,
}

impl Asking for VideoCall {
    type Made = (AskedVideo, VideoJob);

    async fn send(
        &self,
        making: &Making<'_>,
        maker: &MakerOf,
        account: Option<&CredentialId>,
        label: Option<String>,
    ) -> Result<(AskedVideo, VideoJob), MediaFailure> {
        let videos = maker.videos().expect("a video call's maker makes videos");
        let (mut row, span) = making.open_row("video", account);
        let request = VideoRequest { model: making.name.clone(), ..self.request.clone() };
        let http = making.http();
        // The job's bound counts from its ask, the ask's own wait included
        // (`videos.md` § The job).
        let asked_at = tokio::time::Instant::now();
        let answer = videos
            .ask(&http, request.clone())
            .instrument(span.clone())
            .await;
        row.observe(&answer.observed);
        let job = match answer.result {
            Ok(job) => job,
            Err(failure) => {
                row.row().outcome = failure.outcome();
                span.in_scope(|| tracing::info!(outcome = %row.row().outcome, "a video call ended at its ask"));
                row.write().await;
                return Err(failure);
            }
        };
        span.in_scope(|| tracing::info!(job = %job, "a video's job was asked for"));
        row.row().vendor_id = Some(job.clone());
        let shape = videos.shape();
        let asked = AskedVideo {
            job: job.clone(),
            model: making.name.clone(),
            provider: making.route.entry.label.clone(),
            vendor: videos.name().to_owned(),
            account: label.clone(),
            shape: shape.name().to_owned(),
            unsent: videos::unsent(shape, &request).into_iter().map(plugin_option).collect(),
        };
        let tuning = making.shard.services().media_tuning;
        let job = Job {
            maker: maker.clone(),
            http,
            id: job,
            vendor: videos.name().to_owned(),
            label,
            manages: making.manages,
            row,
            span,
            last: None,
            every: tuning.video_reads_every,
            bound: tuning.video_bound,
            deadline: asked_at + tuning.video_bound,
        };
        let steps = futures_util::stream::unfold(job, |mut job| async move {
            let step = job.next().await?;
            Some((step, job))
        });
        Ok((asked, steps.boxed_local()))
    }
}

fn plugin_option(option: VideoOption) -> demi_plugin_interface::VideoOption {
    use demi_plugin_interface::VideoOption as Plugin;
    match option {
        VideoOption::Image => Plugin::Image,
        VideoOption::LastFrame => Plugin::LastFrame,
        VideoOption::Reference => Plugin::Reference,
        VideoOption::Duration => Plugin::Duration,
        VideoOption::Aspect => Plugin::Aspect,
        VideoOption::Resolution => Plugin::Resolution,
        VideoOption::Audio => Plugin::Audio,
        VideoOption::Seed => Plugin::Seed,
    }
}

/// A video's job once the vendor took it: its state read every
/// `every`, each change told, then its video downloaded, all within
/// `bound` of the ask. It ends with its row written once, after the wait
/// its bound covers; dropped before, it cancels the job where the vendor
/// can and writes the cancelled row.
struct Job {
    maker: MakerOf,
    http: demi_provider_common::ProviderHttp,
    /// The vendor's id of the job.
    id: String,
    vendor: String,
    label: Option<String>,
    manages: bool,
    row: OpenRow,
    span: tracing::Span,
    /// The state told last.
    last: Option<VideoStep>,
    every: Duration,
    bound: Duration,
    deadline: tokio::time::Instant,
}

/// What a job's wait came to: a change of state to tell, or its end, with
/// the outcome its row records.
enum Waited {
    Told(VideoStep),
    Ended { outcome: String, result: Result<VideoStep, PortFailure> },
}

impl Job {
    /// The job's next change of state, its video or why it ended; none
    /// once it ended. Only the wait is bounded: an end writes the row
    /// after it, whether or not the bound has passed meanwhile.
    async fn next(&mut self) -> Option<Result<VideoStep, PortFailure>> {
        if !self.row.is_open() {
            return None;
        }
        let span = self.span.clone();
        let (outcome, result) = match tokio::time::timeout_at(self.deadline, self.wait().instrument(span)).await {
            Ok(Waited::Told(step)) => return Some(Ok(step)),
            Ok(Waited::Ended { outcome, result }) => (outcome, result),
            // The job goes on at the vendor, which the call's first line
            // named; the call stops waiting for it.
            Err(_) => (
                "failed".to_owned(),
                Err(refused(MediaRefusal::StillRunning {
                    vendor: self.vendor.clone(),
                    job: self.id.clone(),
                    seconds: self.bound.as_secs(),
                })),
            ),
        };
        self.row.row().outcome = outcome;
        self.span.in_scope(|| tracing::info!(outcome = %self.row.row().outcome, "a video's job ended"));
        self.row.write().await;
        Some(result)
    }

    async fn wait(&mut self) -> Waited {
        let videos = self.maker.videos().expect("a video's job is a video maker's");
        loop {
            tokio::time::sleep(self.every).await;
            let answer = videos.state(&self.http, &self.id).await;
            self.row.observe(&answer.observed);
            let state = match answer.result {
                Ok(state) => state,
                Err(failure) => return self.failed(failure),
            };
            let step = match state {
                JobState::Queued => VideoStep::Queued,
                JobState::Running { progress } => VideoStep::Running { progress },
                JobState::Done(done) => return self.download(&done).await,
                JobState::Failed { message } => {
                    let result = Err(refused(MediaRefusal::JobFailed { vendor: self.vendor.clone(), message }));
                    return Waited::Ended { outcome: "failed".to_owned(), result };
                }
                JobState::Expired => {
                    let result = Err(refused(MediaRefusal::JobExpired { vendor: self.vendor.clone() }));
                    return Waited::Ended { outcome: "failed".to_owned(), result };
                }
            };
            if self.last.as_ref() != Some(&step) {
                self.last = Some(step.clone());
                return Waited::Told(step);
            }
        }
    }

    /// The finished job's video. The cost the vendor reported goes on the
    /// row at once, so a download that then fails, a stopped call or one
    /// past its bound still records what was paid; the seconds once the
    /// file is read.
    async fn download(&mut self, done: &Done) -> Waited {
        let row = self.row.row();
        row.cost = done.cost;
        row.usage = done.usage;
        let videos = self.maker.videos().expect("a video's job is a video maker's");
        let answer = videos.download(&self.http, done).await;
        self.row.observe(&answer.observed);
        let bytes = match answer.result {
            Ok(bytes) => bytes,
            Err(failure) => return self.failed(failure),
        };
        // A file this does not read is the plugin's to refuse; its row
        // keeps no seconds.
        self.row.row().media_seconds = mp4::read(&bytes).ok().map(|facts| facts.seconds);
        let video = MadeVideo {
            bytes: bytes.into(),
            cost: done.cost,
        };
        Waited::Ended { outcome: "answered".to_owned(), result: Ok(VideoStep::Made { video }) }
    }

    fn failed(&self, failure: MediaFailure) -> Waited {
        let outcome = failure.outcome();
        let result = Err(refused(vendor_refusal(failure, self.vendor.clone(), self.label.clone(), self.manages)));
        Waited::Ended { outcome, result }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if !self.row.is_open() {
            return;
        }
        self.span.in_scope(|| tracing::info!("a video's job was stopped"));
        let maker = self.maker.clone();
        let http = self.http.clone();
        let id = self.id.clone();
        // Nothing awaits a stopped call; a vendor that cannot cancel runs
        // the job to its end, as the call's first line said.
        tokio::spawn(
            async move {
                let Some(cancel) = maker.videos().and_then(|videos| videos.cancel(&http, &id)) else {
                    return;
                };
                if let Err(failure) = cancel.await.result {
                    tracing::warn!(?failure, "a stopped video's job was not cancelled at the vendor");
                }
            }
            .instrument(self.span.clone()),
        );
        // The row, still open, is written as cancelled when it drops.
    }
}

/// One call made on an entry: the account it takes under the entry's
/// rotation, and another at once when that one is used up; a key's entry
/// has no account.
struct Making<'a> {
    shard: &'a Shard,
    /// What the model takes, as the call's catalog says.
    catalog: &'a Catalog,
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
        let entry = &self.route.entry;
        let vendor = self.route.media().name().to_owned();
        if !self.route.subscription() {
            return asking
                .send(&self, &self.route.maker, None, None)
                .await
                .map_err(|failure| refused(vendor_refusal(failure, vendor, None, self.manages)));
        }
        let services = self.shard.services();
        let assembly = &services.assembly;
        let rotation = entry.rotation();
        // The accounts the call used up; taking one again means the
        // rotation has no other.
        let mut tried: Vec<CredentialId> = Vec::new();
        let mut last_moment = None;
        loop {
            let used_up = |resets_at| refused(MediaRefusal::UsedUp { vendor: vendor.clone(), resets_at });
            // Another account is sent to only while the entry may still
            // make its kind.
            if !tried.is_empty()
                && current(services, self.shard.user(), self.route).await.map_err(failed)?.is_none()
            {
                return Err(used_up(last_moment));
            }
            let (account, running) = match rotation {
                Rotation::Manual => {
                    let active = entry.active().cloned();
                    (active.ok_or_else(|| refused(MediaRefusal::NoProvider))?, None)
                }
                Rotation::InOrder | Rotation::Balanced => {
                    let (choice, _) = assembly.take_account(entry, &self.model).await.map_err(failed)?;
                    match choice {
                        Some(Choice::Account(running)) if tried.contains(running.account()) => {
                            return Err(used_up(last_moment));
                        }
                        Some(Choice::Account(running)) => (running.account().clone(), Some(running)),
                        Some(Choice::AllUsedUp { earliest }) => return Err(used_up(earliest)),
                        None => unreachable!("a rotating entry takes an account or finds them used up"),
                    }
                }
            };
            let provider = assembly
                .for_account(entry, &account)
                .await
                .map_err(failed)?
                .ok_or_else(|| refused(MediaRefusal::NoProvider))?;
            let now = services.clock.now();
            // A Manual entry whose account a call used up is not asked
            // again until it resets: Automatic goes on with the next entry.
            if rotation == Rotation::Manual && assembly.rotation().is_used_up(&entry.id, &account, now) {
                return Err(used_up(provider.quota().and_then(|quota| quota.used_up_until(now))));
            }
            // The account is named only to whoever manages the entry: on a
            // shared instance it is the administrator's (`providers.md`
            // § Scope).
            let label = if self.manages {
                let record = services.vault.account(entry.id.clone(), account.clone()).await.ok().flatten();
                Some(record.map_or_else(|| account.to_string(), |record| record.label))
            } else {
                None
            };
            let Some(maker) = assembly.maker(entry, &provider, self.route.kind) else {
                return Err(refused(MediaRefusal::NoProvider));
            };
            let sent = asking.send(&self, &maker, Some(&account), label.clone()).await;
            // The call's account counts no longer: its request ended, and a
            // video's job, once asked for, is the vendor's.
            drop(running);
            let failure = match sent {
                Ok(made) => return Ok(made),
                Err(failure) => failure,
            };
            let MediaFailure::UsageLimit { retry_at } = failure else {
                return Err(refused(vendor_refusal(failure, vendor, label, self.manages)));
            };
            let now = services.clock.now();
            let moment = retry_at
                .or_else(|| provider.quota()?.used_up_until(now))
                .filter(|moment| *moment > now);
            let until = moment.unwrap_or_else(|| {
                Timestamp::from_millisecond(now.as_millisecond() + USED_UP_FOR_MS)
                    .expect("half an hour from now is a time")
            });
            // Used up as a turn's failure marks it, whatever the call does
            // next.
            assembly.mark_used_up(&entry.id, &account, until);
            if rotation == Rotation::Manual {
                return Err(used_up(moment));
            }
            tried.push(account);
            last_moment = moment;
        }
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
            request = %request_id,
            agent = self.agent,
            account = account.map(tracing::field::display),
            provider = %entry.id,
            model = %self.name,
        );
        let now = services.clock.now();
        let row = UsageRow {
            user: self.shard.user().clone(),
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
            cost: None,
        };
        (OpenRow { row: Some(row), shard: self.shard.this() }, span)
    }
}

/// What the plugin is told of a call the vendor did not answer as asked;
/// `manages` says whether the user manages the entry.
fn vendor_refusal(failure: MediaFailure, vendor: String, account: Option<String>, manages: bool) -> MediaRefusal {
    match failure {
        MediaFailure::SignIn(failure) if matches!(failure.reason, AuthReason::Store(_)) => {
            MediaRefusal::Failed { message: failure.message() }
        }
        MediaFailure::SignIn(_) => MediaRefusal::Expired { vendor, account },
        MediaFailure::KeyRefused { .. } => MediaRefusal::KeyRefused { vendor, manages },
        MediaFailure::Refused { message, .. } => MediaRefusal::Refused { vendor, message },
        MediaFailure::Shape { takes, .. } => MediaRefusal::Shape { vendor, takes },
        MediaFailure::Unanswered { cause, .. } => MediaRefusal::Unanswered { vendor, cause },
        MediaFailure::UsageLimit { retry_at } => MediaRefusal::UsedUp { vendor, resets_at: retry_at },
    }
}

/// The ledger row of a call open now, written as cancelled if the call is
/// dropped before it ends.
struct OpenRow {
    row: Option<UsageRow>,
    shard: Rc<Shard>,
}

impl OpenRow {
    fn is_open(&self) -> bool {
        self.row.is_some()
    }

    fn row(&mut self) -> &mut UsageRow {
        self.row.as_mut().expect("a call's row is open until it is written")
    }

    /// Keeps the windows of the account's quota a response reported.
    fn observe(&mut self, observed: &[QuotaWindow]) {
        if observed.is_empty() {
            return;
        }
        merge_windows(self.row().quota.get_or_insert_with(Vec::new), observed);
    }

    /// Writes the row, ended now.
    async fn write(&mut self) {
        let mut row = self.row.take().expect("a call's row is written once");
        row.ended_at = self.shard.services().clock.now();
        if let Err(error) = self.shard.services().control.append_usage(row).await {
            // The row is lost; what the call made still comes back
            // (`usage-and-quota.md` § Usage ledger).
            tracing::warn!(error = &error as &dyn std::error::Error, "a usage ledger row was not written");
        }
    }
}

impl Drop for OpenRow {
    fn drop(&mut self) {
        if let Some(mut row) = self.row.take() {
            row.ended_at = self.shard.services().clock.now();
            let control = self.shard.services().control.clone();
            // Nothing awaits a dropped call any more.
            tokio::spawn(async move {
                if let Err(error) = control.append_usage(row).await {
                    tracing::warn!(error = &error as &dyn std::error::Error, "a usage ledger row was not written");
                }
            });
        }
    }
}
