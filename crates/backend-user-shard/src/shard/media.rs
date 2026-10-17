//! The Providers service of the plugins' port for generated media
//! (`media.md`, `plugins.md` § The contract): the models of one kind the
//! user's entries offer, each with every fact the catalog of media models
//! states and the entries that offer it, and one image made through an
//! entry's account or key.
//!
//! Which entry makes a call (`media.md` § Which entry makes a call): the
//! one `--provider` names, which fails the call when it cannot; otherwise
//! the first that can make the model now, the subscriptions before the
//! keys, each in the order of Settings › Providers. An entry can when the
//! user has it on, its switch for the kind is on, it is signed in or has
//! its key, it offers the model and an account of it is not used up. An
//! entry whose last account is used up during the call hands it on to the
//! next; a refusal of the request is never handed on. Within an entry the
//! call takes the account under the entry's rotation, as a new agent takes
//! one at its first request, and another at once when that one is used up.
//! The provider makes the image, renewing its account's sign-in, and each
//! call is a usage ledger row with the purpose `image`. What the agent
//! reads of it, the plugin words.

use demi_backend_database::usage::{UsageRow, merge_windows};
use demi_backend_providers::llm::assembly::{AssemblyError, ImageMakerOf};
use demi_backend_providers::llm::rotation::{Choice, USED_UP_FOR_MS};
use demi_backend_providers::vault::entries::{EntryCredential, ProviderEntry};
use demi_plugin_interface::{
    CustomSizeRule, EntryStatus, ImageAsk, ImageFacts, ImageRefusal, ImageUnlisted, MadeImage, MediaListing,
    MediaModel, MediaPrice, ModelEntry, PortFailure, PortRefusal, PriceRate, Unable,
};
use demi_provider_common::credentials::AuthReason;
use demi_provider_common::images::{
    Aspect, ImageFailure, ImageMaker, ImageModelInfo, ImageRequest, ImageVendor, InputImage,
};
use demi_provider_common::media_catalog::{Catalog, Currency, MediaKind, Modality, PriceUnit};
use demi_host_interface::{JobCaller, PortError};
use demi_shared_types::{Model, Rotation, Timestamp, TurnId};
use demi_web_api_protocol::ids::{ConversationId, CredentialId};
use demi_web_api_protocol::providers::EntryImageModel;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::Shard;
use super::provider_commands::named;
use crate::services::Services;
use demi_backend_host_access::root_of;

/// An entry that makes images, with what makes them for its active
/// account, in the order a call tries the entries.
struct Route {
    entry: ProviderEntry,
    maker: ImageMakerOf,
    /// The user turned the entry off in Settings › Providers.
    hidden: bool,
}

impl Route {
    fn images(&self) -> &dyn ImageMaker {
        self.maker.images()
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

fn refused(refusal: ImageRefusal) -> PortFailure {
    PortFailure::Refused(PortRefusal::Image { refusal })
}

fn failed(error: impl ToString) -> PortFailure {
    refused(ImageRefusal::Failed { message: error.to_string() })
}

/// The user's entries that make images, in the order a call tries them:
/// the subscriptions before the keys, each in the order of Settings ›
/// Providers. An entry that lacks what its provider is built from, such as
/// a key, offers nothing yet and is left out.
async fn routes(services: &Services, user: &demi_web_api_protocol::ids::UserId) -> Result<Vec<Route>, AssemblyError> {
    let preferences = services.control.preferences(user.clone()).await?;
    let owner = services.vault.owner_for(user).await?;
    let mut routes = Vec::new();
    for entry in services.vault.entries(owner).await? {
        if services.assembly.media_shapes(&entry).image.is_none() {
            continue;
        }
        let Some(route) = route(services, entry, &preferences).await? else {
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
) -> Result<Option<Route>, AssemblyError> {
    let provider = match services.assembly.provider_for(&entry).await {
        Ok(provider) => provider,
        // An entry that lacks what it needs makes nothing yet.
        Err(AssemblyError::Incomplete) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(maker) = services.assembly.image_maker(&entry, &provider) else {
        return Ok(None);
    };
    Ok(Some(Route {
        hidden: preferences.hides_provider(&entry.id),
        entry,
        maker,
    }))
}

/// Entry `route` as it is now, while it may still make images: it exists,
/// the user has it on, its Image Generation is on, and it is signed in or
/// has its key, read again right before a request is sent, since any of
/// them may have changed while an earlier entry's request was under way.
async fn current(services: &Services, user: &demi_web_api_protocol::ids::UserId, route: &Route) -> Result<Option<Route>, AssemblyError> {
    let Some(entry) = services.vault.entry(route.entry.id.clone()).await? else {
        return Ok(None);
    };
    let preferences = services.control.preferences(user.clone()).await?;
    let Some(now) = self::route(services, entry, &preferences).await? else {
        return Ok(None);
    };
    let usable = !now.hidden
        && now.entry.generates(demi_shared_types::MediaKind::Image)
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
    if !entry.generates(demi_shared_types::MediaKind::Image) {
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

/// Each entry's image models, by entry id, newest first, for the settings
/// page's Images section (`providers.md` § Provider management): whether or
/// not its Image Generation is on, and an alias left out beside the model
/// it names. An entry whose models could not be listed has none here; the
/// agent's listing says why.
pub async fn entry_image_models(
    services: &Services,
    user: &demi_web_api_protocol::ids::UserId,
) -> Result<std::collections::HashMap<String, Vec<EntryImageModel>>, AssemblyError> {
    let routes = routes(services, user).await?;
    let catalog = services.media.current();
    let https: Vec<_> = routes.iter().map(|route| route.http(services, services.assembly.http())).collect();
    let listings =
        futures_util::future::join_all(routes.iter().zip(&https).map(|(route, http)| route.images().models(http, &catalog))).await;
    let mut entries = std::collections::HashMap::new();
    for (route, listing) in routes.iter().zip(listings) {
        let listed = listing.unwrap_or_default();
        let vendors: Vec<ImageVendor> = route.images().vendor().into_iter().collect();
        let mut models: Vec<EntryImageModel> = listed
            .iter()
            .filter(|model| {
                let named = vendors.iter().find_map(|vendor| catalog.model(vendor.id(), &model.id));
                !named
                    .and_then(|known| known.alias_of.as_deref())
                    .is_some_and(|target| listed.iter().any(|other| other.id == target))
            })
            .map(|model| {
                let described = described(&catalog, model, &vendors, Vec::new());
                EntryImageModel {
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

/// Whether the user manages the entries they make images with, and so may
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
) -> (Vec<Vec<ImageModelInfo>>, Vec<ImageUnlisted>, Option<(String, ImageFailure)>) {
    let mut offered = Vec::new();
    let mut unlisted = Vec::new();
    let mut first_failure = None;
    for route in routes {
        match route.images().models(&route.http(shard.services(), shard.http()), catalog).await {
            Ok(models) => offered.push(models),
            Err(failure) => {
                let vendor = route.images().name().to_owned();
                unlisted.push(ImageUnlisted {
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

/// Why no entry makes images: there is none, or every one is off.
fn none_made(routes: &[Route]) -> ImageRefusal {
    let off: Vec<String> = routes
        .iter()
        .filter(|route| !route.entry.generates(demi_shared_types::MediaKind::Image))
        .map(|route| route.entry.label.clone())
        .collect();
    if routes.is_empty() || off.len() < routes.len() {
        ImageRefusal::NoProvider
    } else {
        ImageRefusal::AllOff { providers: off }
    }
}

/// The names of `id`'s model, `id` first: the model an alias names and
/// every alias of that model, as the catalog states them; `id` alone for a
/// model the catalog does not know. An entry that lists any of them offers
/// the model (`media.md` § What the agent is told).
fn model_names(catalog: &Catalog, id: &str) -> Vec<String> {
    let Some(known) = catalog.models.iter().find(|model| model.kind == MediaKind::Image && model.id == id) else {
        return vec![id.to_owned()];
    };
    let canonical = known.alias_of.clone().unwrap_or_else(|| known.id.clone());
    let mut names = vec![id.to_owned()];
    let group = catalog.models.iter().filter(|model| {
        model.kind == MediaKind::Image
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

/// The model a name names: the model itself for an alias.
fn canonical(catalog: &Catalog, id: &str) -> String {
    catalog
        .models
        .iter()
        .find(|model| model.kind == MediaKind::Image && model.id == id)
        .and_then(|model| model.alias_of.clone())
        .unwrap_or_else(|| id.to_owned())
}

/// The name of `names` that `listing` offers, the first in their order.
fn listed_name<'a>(listing: &[ImageModelInfo], names: &'a [String]) -> Option<&'a str> {
    names
        .iter()
        .find(|name| listing.iter().any(|model| &model.id == *name))
        .map(String::as_str)
}

/// The image models of the user's entries, newest first, each with what
/// the catalog states of it and the entries that offer it in the order a
/// call tries them, with why each would or would not make it now
/// (`media.md` § What the agent is told); a model no entry that is on
/// offers is not listed.
pub(crate) async fn image_models(shard: &Shard) -> Result<MediaListing, PortFailure> {
    let services = shard.services();
    let routes = routes(services, shard.user()).await.map_err(failed)?;
    let manages = manages(shard).await?;
    let catalog = services.media.current();
    let (offered, unlisted, _) = offerings(shard, &catalog, &routes, manages).await;
    // An alias is a name of a model, not a model: a model is one row, with
    // its aliases among its facts, and every entry that offers any of its
    // names (`media.md` § What the agent is told).
    let mut ids: Vec<ImageModelInfo> = Vec::new();
    for listing in &offered {
        for model in listing {
            let id = canonical(&catalog, &model.id);
            if ids.iter().any(|known| known.id == id) {
                continue;
            }
            let listed = listing
                .iter()
                .find(|listed| listed.id == id)
                .cloned()
                .unwrap_or_else(|| if model.id == id { model.clone() } else { ImageModelInfo::id(&id) });
            ids.push(listed);
        }
    }
    let mut models = Vec::new();
    for listed in &ids {
        let names = model_names(&catalog, &listed.id);
        let mut entries = Vec::new();
        let mut taken = false;
        let mut vendors = Vec::new();
        for (route, listing) in routes.iter().zip(&offered) {
            let Some(name) = listed_name(listing, &names) else {
                continue;
            };
            vendors.extend(route.images().vendor());
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
                confirmed: route.images().confirms_model(),
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
            models.push(described(&catalog, listed, &vendors, entries));
        }
    }
    // ISO dates sort as text, newest first and those without one last; a
    // stable sort keeps the listings' order within one date.
    models.sort_by(|a, b| b.released.cmp(&a.released));
    let none = (models.is_empty() && unlisted.is_empty()).then(|| none_made(&routes));
    Ok(MediaListing { models, unlisted, none })
}

/// `listed` with what the catalog states of it, for the first of `vendors`
/// whose catalog entry it is, else what its listing says.
fn described(catalog: &Catalog, listed: &ImageModelInfo, vendors: &[ImageVendor], entries: Vec<ModelEntry>) -> MediaModel {
    let known = vendors.iter().find_map(|vendor| {
        catalog
            .model(vendor.id(), &listed.id)
            .filter(|model| model.kind == MediaKind::Image)
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
    tokio::select! {
        made = make(shard, conversation, caller, ask) => made,
        () = cancel.cancelled() => Err(PortError::Ended("the call was stopped".into()).into()),
    }
}

async fn make(
    shard: &Shard,
    conversation: &ConversationId,
    caller: Option<&JobCaller>,
    ask: ImageAsk,
) -> Result<MadeImage, PortFailure> {
    let services = shard.services();
    let routes = routes(services, shard.user()).await.map_err(failed)?;
    let model = ask.model.clone();
    let request = ImageRequest {
        model: model.clone(),
        prompt: ask.prompt,
        aspect: ask.aspect.map(|aspect| Aspect {
            width: aspect.width,
            height: aspect.height,
        }),
        transparent: ask.transparent,
        images: ask
            .images
            .into_iter()
            .map(|image| InputImage {
                media_type: image.kind.media_type(),
                bytes: image.bytes.into_bytes(),
            })
            .collect(),
    };
    let agent = caller.and_then(|caller| {
        let node = shard.agent().node(&root_of(conversation), &caller.node)?;
        Some(node.record().number)
    });
    let manages = manages(shard).await?;
    let catalog = services.media.current();
    let (offered, _, unread) = offerings(shard, &catalog, &routes, manages).await;
    // Each entry that offers any name of the model, with the name it is
    // sent: the one asked when it lists it, else another of the model's.
    let names = model_names(&catalog, &model);
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
    let tried: Vec<&Route> = match &ask.provider {
        Some(name) => {
            let entries: Vec<ProviderEntry> = routes.iter().map(|route| route.entry.clone()).collect();
            let entry = match named(&entries, name) {
                Ok(entry) => entry.clone(),
                Err(message) => {
                    // An entry that makes no images is named as one that
                    // does not offer the model.
                    let owner = services.vault.owner_for(shard.user()).await.map_err(failed)?;
                    let all = services.vault.entries(owner).await.map_err(failed)?;
                    let Ok(other) = named(&all, name) else {
                        return Err(failed(message));
                    };
                    return Err(refused(ImageRefusal::Cannot {
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
                return Err(refused(ImageRefusal::Cannot {
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
        let request = ImageRequest { model: name.clone(), ..request.clone() };
        let making = Making {
            shard,
            catalog: &catalog,
            manages,
            turn: caller.and_then(|caller| caller.turn.clone()),
            route: &route,
            request: &request,
            agent,
            conversation: conversation.clone(),
            model: model_of(&name),
        };
        match making.run().await {
            // Used up: the next entry goes on at once, and the first one's
            // limit is told if none is left.
            Err(failure @ PortFailure::Refused(PortRefusal::Image { refusal: ImageRefusal::UsedUp { .. } })) => {
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
        let vendor = route.images().name().to_owned();
        return Err(refused(match standing {
            Standing::UsedUp(resets_at) => ImageRefusal::UsedUp { vendor, resets_at: *resets_at },
            Standing::Off => ImageRefusal::Off { model, provider: route.entry.label.clone() },
            Standing::NotSignedIn | Standing::TurnedOff | Standing::Ready => ImageRefusal::Cannot {
                provider: route.entry.label.clone(),
                model,
                why: standing.unable().unwrap_or(Unable::NotOffered),
                can: Vec::new(),
            },
        }));
    }
    let makes = routes.iter().any(|route| route.entry.generates(demi_shared_types::MediaKind::Image));
    if !makes {
        return Err(refused(none_made(&routes)));
    }
    if let Some((vendor, failure)) = unread {
        return Err(refused(vendor_refusal(failure, vendor, None, manages)));
    }
    Err(refused(ImageRefusal::UnknownModel { model }))
}

/// One image made on an entry: the account it takes under the entry's
/// rotation, and another at once when that one is used up; a key's entry
/// has no account.
struct Making<'a> {
    shard: &'a Shard,
    /// What the model takes, as the call's catalog says.
    catalog: &'a Catalog,
    route: &'a Route,
    request: &'a ImageRequest,
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
    async fn run(self) -> Result<MadeImage, PortFailure> {
        let entry = &self.route.entry;
        if !self.route.subscription() {
            let maker = self.route.images();
            return match self.call(maker, None).await {
                Ok(image) => Ok(self.made(image, maker, None)),
                Err(failure) => Err(refused(vendor_refusal(failure, maker.name().to_owned(), None, self.manages))),
            };
        }
        let services = self.shard.services();
        let assembly = &services.assembly;
        let rotation = entry.rotation();
        let vendor = self.route.images().name().to_owned();
        // The accounts the call used up; taking one again means the
        // rotation has no other.
        let mut tried: Vec<CredentialId> = Vec::new();
        let mut last_moment = None;
        loop {
            let used_up = |resets_at| refused(ImageRefusal::UsedUp { vendor: vendor.clone(), resets_at });
            // Another account is sent to only while the entry may still
            // make images.
            if !tried.is_empty()
                && current(services, self.shard.user(), self.route).await.map_err(failed)?.is_none()
            {
                return Err(used_up(last_moment));
            }
            let (account, running) = match rotation {
                Rotation::Manual => {
                    let active = entry.active().cloned();
                    (active.ok_or_else(|| refused(ImageRefusal::NoProvider))?, None)
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
                .ok_or_else(|| refused(ImageRefusal::NoProvider))?;
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
            let Some(maker) = services.assembly.image_maker(entry, &provider) else {
                return Err(refused(ImageRefusal::NoProvider));
            };
            let maker = maker.images();
            let made = self.call(maker, Some(&account)).await;
            // The call's account counts no longer: it made its one image.
            drop(running);
            let failure = match made {
                Ok(image) => return Ok(self.made(image, maker, label)),
                Err(failure) => failure,
            };
            let ImageFailure::UsageLimit { retry_at } = failure else {
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

    /// The image `maker` made, as the plugin is told it.
    fn made(
        &self,
        image: demi_provider_common::images::MadeImage,
        maker: &dyn ImageMaker,
        account: Option<String>,
    ) -> MadeImage {
        MadeImage {
            bytes: image.bytes.into(),
            model: self.request.model.clone(),
            provider: self.route.entry.label.clone(),
            vendor: maker.name().to_owned(),
            account,
            confirmed: maker.confirms_model(),
        }
    }

    /// One call to the vendor on `account`, or with the entry's key, and
    /// its ledger row; a call dropped before its answer is a cancelled row.
    async fn call(
        &self,
        maker: &dyn ImageMaker,
        account: Option<&CredentialId>,
    ) -> Result<demi_provider_common::images::MadeImage, ImageFailure> {
        let services = self.shard.services();
        let entry = &self.route.entry;
        let request_id = uuid::Uuid::new_v4().to_string();
        let span = tracing::info_span!(
            "image_call",
            request = %request_id,
            agent = self.agent,
            account = account.map(tracing::field::display),
            provider = %entry.id,
            model = %self.request.model,
        );
        let now = services.clock.now();
        let row = OpenRow(
            Some(UsageRow {
                user: self.shard.user().clone(),
                conversation: self.conversation.clone(),
                agent: self.agent,
                turn: self.turn.as_ref().map(ToString::to_string),
                purpose: "image",
                provider: entry.id.clone(),
                account: account.cloned(),
                model: self.request.model.clone(),
                request: request_id,
                vendor_id: None,
                usage: None,
                cache_write_1h_tokens: 0,
                outcome: "cancelled".to_owned(),
                quota: None,
                started_at: now,
                ended_at: now,
            }),
            self.shard,
        );
        let answer = maker
            .make(&self.route.http(services, self.shard.http()), self.catalog, self.request.clone())
            .instrument(span.clone())
            .await;
        let made = answer.result;
        let mut ended = row.end();
        ended.ended_at = services.clock.now();
        if !answer.observed.is_empty() {
            let mut kept = Vec::new();
            merge_windows(&mut kept, &answer.observed);
            ended.quota = Some(kept);
        }
        match &made {
            Ok(image) => {
                ended.outcome = "answered".to_owned();
                ended.usage = image.usage;
                ended.vendor_id = image.vendor_id.clone();
            }
            Err(failure) => ended.outcome = failure.outcome(),
        }
        span.in_scope(|| tracing::info!(outcome = %ended.outcome, "an image call ended"));
        if let Err(error) = services.control.append_usage(ended).await {
            // The row is lost; the image still came back
            // (`usage-and-quota.md` § Usage ledger).
            tracing::warn!(error = &error as &dyn std::error::Error, "a usage ledger row was not written");
        }
        made
    }
}

/// What the plugin is told of a call the vendor did not answer with an
/// image; `manages` says whether the user manages the entry.
fn vendor_refusal(failure: ImageFailure, vendor: String, account: Option<String>, manages: bool) -> ImageRefusal {
    match failure {
        ImageFailure::SignIn(failure) if matches!(failure.reason, AuthReason::Store(_)) => {
            ImageRefusal::Failed { message: failure.message() }
        }
        ImageFailure::SignIn(_) => ImageRefusal::Expired { vendor, account },
        ImageFailure::KeyRefused { .. } => ImageRefusal::KeyRefused { vendor, manages },
        ImageFailure::Refused { message, .. } => ImageRefusal::Refused { vendor, message },
        ImageFailure::Shape { takes, .. } => ImageRefusal::Shape { vendor, takes },
        ImageFailure::Unanswered { cause, .. } => ImageRefusal::Unanswered { vendor, cause },
        ImageFailure::UsageLimit { retry_at } => ImageRefusal::UsedUp { vendor, resets_at: retry_at },
    }
}

/// The row of a call open now, written as cancelled if the call is dropped
/// before its answer.
struct OpenRow<'a>(Option<UsageRow>, &'a Shard);

impl OpenRow<'_> {
    /// The row of the call that ended.
    fn end(mut self) -> UsageRow {
        self.0.take().expect("a call ends once")
    }
}

impl Drop for OpenRow<'_> {
    fn drop(&mut self) {
        if let Some(mut row) = self.0.take() {
            row.ended_at = self.1.services().clock.now();
            let control = self.1.services().control.clone();
            // Nothing awaits a dropped call any more.
            tokio::spawn(async move {
                if let Err(error) = control.append_usage(row).await {
                    tracing::warn!(error = &error as &dyn std::error::Error, "a usage ledger row was not written");
                }
            });
        }
    }
}
