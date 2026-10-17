//! The Providers service of the plugins' port (`images.md`, `plugins.md`
//! § The contract): the image models of the user's entries by vendor, and
//! one image made through an entry's account or key. The `--model` names
//! the vendor; the service takes the vendor's entry the user chose while it
//! can make the model, else Automatic's: the subscriptions first, then the
//! keys, each in the order of Settings › Providers (`images.md` § Which
//! entry makes an image). Within an entry it takes the account under the
//! entry's rotation, as a new agent takes one at its first request; an
//! account used up moves the call to another at once, and an entry used up
//! to the next entry, as a turn's failure would. The provider makes the
//! image, renewing its account's sign-in, and each call is a usage ledger
//! row with the purpose `image`. What the agent reads of it, the plugin
//! words.

use std::collections::BTreeMap;
use std::sync::Arc;

use demi_backend_database::usage::{UsageRow, merge_windows};
use demi_backend_providers::llm::assembly::AssemblyError;
use demi_backend_providers::llm::rotation::{Choice, USED_UP_FOR_MS};
use demi_backend_providers::vault::entries::{EntryCredential, ProviderEntry};
use demi_plugin_interface::{
    ImageAsk, ImageEntry, ImageModel, ImageRefusal, ImageUnlisted, ImageVendorModels, MadeImage, PortFailure,
    PortRefusal,
};
use demi_provider_common::Provider;
use demi_provider_common::credentials::AuthReason;
use demi_provider_common::images::{
    Aspect, ImageFailure, ImageMaker, ImageModelInfo, ImageRequest, ImageVendor, InputImage, facts,
};
use demi_host_interface::{JobCaller, PortError};
use demi_shared_types::{Model, Rotation, Timestamp, TurnId};
use demi_web_api_protocol::ids::{ConversationId, CredentialId, ProviderId, UserId};
use demi_web_api_protocol::settings::Preferences;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::Shard;
use crate::services::Services;
use demi_backend_host_access::root_of;

/// An entry that makes images, with the provider of its active account or
/// its key.
struct Maker {
    entry: ProviderEntry,
    provider: Arc<dyn Provider>,
}

impl Maker {
    fn images(&self) -> &dyn ImageMaker {
        self.provider.images().expect("a maker's provider makes images")
    }

    fn vendor(&self) -> ImageVendor {
        self.images().vendor()
    }

    fn subscription(&self) -> bool {
        matches!(self.entry.credential, EntryCredential::Subscription { .. })
    }
}

fn refused(refusal: ImageRefusal) -> PortFailure {
    PortFailure::Refused(PortRefusal::Image { refusal })
}

fn failed(error: impl ToString) -> PortFailure {
    refused(ImageRefusal::Failed { message: error.to_string() })
}

/// The user's entries whose providers make images, in the order of
/// Settings › Providers: each entry the user has on, signed in or with its
/// key, whose provider makes images; `on` those whose Image Generation is
/// on, `off` the others (`providers.md` § Provider management).
struct Makers {
    on: Vec<Maker>,
    off: Vec<Maker>,
}

impl Makers {
    /// Why none makes images: every one that could is off, or there is none.
    fn none(&self) -> ImageRefusal {
        if self.off.is_empty() {
            ImageRefusal::NoProvider
        } else {
            ImageRefusal::AllOff {
                providers: self.off.iter().map(|maker| maker.entry.label.clone()).collect(),
            }
        }
    }

    /// The entries that may make `vendor`'s images, in the order a call
    /// tries them: the one `chosen` while it is among them, then
    /// Automatic's, the subscriptions before the keys.
    fn candidates(&self, vendor: ImageVendor, chosen: Option<&ProviderId>) -> Vec<&Maker> {
        let mut automatic: Vec<&Maker> = self.on.iter().filter(|maker| maker.vendor() == vendor).collect();
        // A stable sort keeps the order of Settings › Providers within each
        // kind.
        automatic.sort_by_key(|maker| !maker.subscription());
        if let Some(index) = automatic.iter().position(|maker| Some(&maker.entry.id) == chosen) {
            let first = automatic.remove(index);
            automatic.insert(0, first);
        }
        automatic
    }
}

async fn makers(services: &Services, user: &UserId, preferences: &Preferences) -> Result<Makers, AssemblyError> {
    let mut makers = Makers { on: Vec::new(), off: Vec::new() };
    let owner = services.vault.owner_for(user).await?;
    for entry in services.vault.entries(owner).await? {
        let Some(maker) = maker(services, preferences, entry).await? else {
            continue;
        };
        if maker.entry.image_generation { makers.on.push(maker) } else { makers.off.push(maker) }
    }
    Ok(makers)
}

/// `entry` as a maker of images, with its provider: none while the user
/// has it off, it is not signed in or has no key, or its provider makes
/// none.
async fn maker(services: &Services, preferences: &Preferences, entry: ProviderEntry) -> Result<Option<Maker>, AssemblyError> {
    let usable = match &entry.credential {
        EntryCredential::Subscription { active, .. } => active.is_some(),
        EntryCredential::ApiKey(config) => config.api_key.is_some(),
    };
    if !usable || preferences.hides_provider(&entry.id) {
        return Ok(None);
    }
    let provider = match services.assembly.provider_for(&entry).await {
        Ok(provider) => provider,
        // An entry that lacks what it needs makes nothing yet.
        Err(AssemblyError::Incomplete) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(provider.images().is_some().then_some(Maker { entry, provider }))
}

/// Entry `id` as it is now, when it may still make images: it exists, the
/// user has it on, its Image Generation is on, and it is signed in or has
/// its key, read again right before a request is sent, since any of them
/// may have changed while an earlier entry's request was under way.
async fn current(services: &Services, user: &UserId, id: &ProviderId) -> Result<Option<Maker>, AssemblyError> {
    let Some(entry) = services.vault.entry(id.clone()).await? else {
        return Ok(None);
    };
    if !entry.image_generation {
        return Ok(None);
    }
    let preferences = services.control.preferences(user.clone()).await?;
    maker(services, &preferences, entry).await
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

/// Whether every account of `maker` is known to be used up for `model`
/// now. A key's limit is known only from a call's refusal.
async fn used_up(services: &Services, maker: &Maker, model: &str) -> Result<bool, AssemblyError> {
    let assembly = &services.assembly;
    let entry = &maker.entry;
    match entry.rotation() {
        _ if !maker.subscription() => Ok(false),
        Rotation::Manual => Ok(entry
            .active()
            .is_some_and(|active| assembly.rotation().is_used_up(&entry.id, active, assembly.now()))),
        Rotation::InOrder | Rotation::Balanced => {
            let (choice, _) = assembly.take_account(entry, &model_of(model)).await?;
            // An account taken only to look counts no longer once dropped.
            Ok(matches!(choice, Some(Choice::AllUsedUp { .. })))
        }
    }
}

/// The entry Automatic takes now for each vendor an entry of `user` makes
/// images for, or none when none can: the first entry Automatic would try,
/// judged by all but the model, its switch, its sign-in or key and its
/// limit, the subscriptions before the keys, as Settings › Agent shows
/// beside each vendor's *Automatic* (`images.md` § Which entry makes an
/// image).
pub(crate) async fn automatic_entries(
    services: &Services,
    user: &UserId,
) -> Result<BTreeMap<ImageVendor, Option<ProviderId>>, AssemblyError> {
    let preferences = services.control.preferences(user.clone()).await?;
    let makers = makers(services, user, &preferences).await?;
    let mut automatic = BTreeMap::new();
    for maker in makers.on.iter().chain(&makers.off) {
        automatic.insert(maker.vendor(), None);
    }
    for (vendor, taken) in &mut automatic {
        for maker in makers.candidates(*vendor, None) {
            // The row names no model: an entry is judged by all but the
            // model, so a window scoped to one model does not count.
            if !used_up(services, maker, "").await? {
                *taken = Some(maker.entry.id.clone());
                break;
            }
        }
    }
    Ok(automatic)
}

/// `maker`'s entry as a listing names it: its active account's label to a
/// user who manages it.
async fn entry_named(shard: &Shard, maker: &Maker, manages: bool) -> ImageEntry {
    let account = match maker.entry.active() {
        Some(active) if manages => {
            let record = shard.services().vault.account(maker.entry.id.clone(), active.clone()).await.ok().flatten();
            Some(record.map_or_else(|| active.to_string(), |record| record.label))
        }
        _ => None,
    };
    ImageEntry {
        provider: maker.entry.label.clone(),
        provider_id: maker.entry.id.as_str().to_owned(),
        account,
        confirmed: maker.images().confirms_model(),
    }
}

/// Whether the user manages the entries they make images with, and so may
/// read their accounts and check their keys.
async fn manages(shard: &Shard) -> Result<bool, PortFailure> {
    let user = shard.user();
    Ok(shard.services().vault.owner_for(user).await.map_err(failed)? == *user)
}

/// The image models of the user's entries whose Image Generation is on,
/// by vendor, each group headed by the entry its models go through now and
/// newest first by release date (`images.md` § Commands); when every entry
/// that could make images is off, the refusal that says so.
pub(crate) async fn image_models(shard: &Shard) -> Result<Vec<ImageVendorModels>, PortFailure> {
    let preferences = shard.services().control.preferences(shard.user().clone()).await.map_err(failed)?;
    let makers = makers(shard.services(), shard.user(), &preferences).await.map_err(failed)?;
    if makers.on.is_empty() && !makers.off.is_empty() {
        return Err(refused(makers.none()));
    }
    let manages = manages(shard).await?;
    let http = shard.http();
    let mut groups = Vec::new();
    for vendor in ImageVendor::ALL {
        let candidates = makers.candidates(vendor, preferences.image_entries.get(&vendor));
        if candidates.is_empty() {
            continue;
        }
        // Each entry's models, and whether it can make them now, in the
        // order a call tries the entries.
        let mut listed: Vec<(&Maker, Vec<ImageModelInfo>, bool)> = Vec::new();
        let mut unlisted = Vec::new();
        for maker in candidates.iter().copied() {
            match maker.images().models(http).await {
                Ok(models) => {
                    let spent = match models.first() {
                        Some(model) => used_up(shard.services(), maker, &model.id).await.map_err(failed)?,
                        None => false,
                    };
                    listed.push((maker, models, spent));
                }
                Err(failure) => unlisted.push(ImageUnlisted {
                    provider: maker.entry.label.clone(),
                    refusal: vendor_refusal(failure, maker.images().name().to_owned(), None, manages),
                }),
            }
        }
        let heading = listed
            .iter()
            .find(|(_, _, spent)| !spent)
            .or(listed.first())
            .map_or(candidates[0], |(maker, _, _)| *maker);
        let through = entry_named(shard, heading, manages).await;
        let mut models: Vec<ImageModel> = Vec::new();
        for (maker, offered, _) in &listed {
            for model in offered {
                if models.iter().any(|known| known.id == model.id) {
                    continue;
                }
                // The entry a call takes for it: the first that offers it and
                // is not used up, else this first one that offers it.
                let taker = listed
                    .iter()
                    .find(|(_, offered, spent)| !spent && offered.iter().any(|other| other.id == model.id))
                    .map_or(*maker, |(taker, _, _)| *taker);
                let through = if taker.entry.id == heading.entry.id {
                    None
                } else {
                    Some(entry_named(shard, taker, manages).await)
                };
                models.push(ImageModel {
                    id: model.id.clone(),
                    released: model.released.clone(),
                    description: model.description.clone(),
                    takes: model.takes.clone(),
                    through,
                });
            }
        }
        // ISO dates sort as text, newest first and those without one last; a
        // stable sort keeps the listing's order within one date.
        models.sort_by(|a, b| b.released.cmp(&a.released));
        groups.push(ImageVendorModels {
            vendor: vendor.name().to_owned(),
            vendor_id: vendor.id().to_owned(),
            through,
            models,
            unlisted,
        });
    }
    Ok(groups)
}

/// Whether `maker` offers `model`, by its list; a list that could not be
/// read is kept in `unread` and offers nothing.
async fn offers(shard: &Shard, maker: &Maker, model: &str, unread: &mut Option<(String, ImageFailure)>) -> bool {
    match maker.images().models(shard.http()).await {
        Ok(models) => models.iter().any(|offered| offered.id == model),
        Err(failure) => {
            unread.get_or_insert((maker.images().name().to_owned(), failure));
            false
        }
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
    let preferences = shard.services().control.preferences(shard.user().clone()).await.map_err(failed)?;
    let makers = makers(shard.services(), shard.user(), &preferences).await.map_err(failed)?;
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
    // The vendor whose table names the model is tried first: a model is
    // one vendor's, and the others' keys need not be listed.
    let mut vendors = ImageVendor::ALL;
    vendors.sort_by_key(|vendor| !facts(*vendor).iter().any(|known| known.id == model));
    let mut spent: Option<PortFailure> = None;
    let mut unread = None;
    for vendor in vendors {
        let mut found = false;
        for candidate in makers.candidates(vendor, preferences.image_entries.get(&vendor)) {
            if !offers(shard, candidate, &model, &mut unread).await {
                continue;
            }
            found = true;
            // The entry as it is now: one turned off, deleted or signed out
            // while an earlier entry's request was under way is not sent
            // to, and its key is the one it has now.
            let Some(maker) = current(shard.services(), shard.user(), &candidate.entry.id).await.map_err(failed)? else {
                continue;
            };
            let making = Making {
                shard,
                manages,
                turn: caller.and_then(|caller| caller.turn.clone()),
                maker: &maker,
                request: &request,
                agent,
                conversation: conversation.clone(),
                model: model_of(&model),
            };
            match making.run().await {
                // Used up: the next entry Automatic takes goes on at once,
                // and the first one's limit is told if none is left.
                Err(failure @ PortFailure::Refused(PortRefusal::Image { refusal: ImageRefusal::UsedUp { .. } })) => {
                    spent.get_or_insert(failure);
                }
                made => return made,
            }
        }
        if found {
            break;
        }
    }
    if let Some(spent) = spent {
        return Err(spent);
    }
    let mut unread_off = None;
    for maker in &makers.off {
        if offers(shard, maker, &model, &mut unread_off).await {
            return Err(refused(ImageRefusal::Off {
                model,
                provider: maker.entry.label.clone(),
            }));
        }
    }
    if makers.on.is_empty() {
        return Err(refused(makers.none()));
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
    maker: &'a Maker,
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
        let entry = &self.maker.entry;
        if !self.maker.subscription() {
            let maker = self.maker.images();
            return match self.call(maker, None).await {
                Ok(image) => Ok(self.made(image, maker, None)),
                Err(failure) => Err(refused(vendor_refusal(failure, maker.name().to_owned(), None, self.manages))),
            };
        }
        let services = self.shard.services();
        let assembly = &services.assembly;
        let rotation = entry.rotation();
        let vendor = self.maker.images().name().to_owned();
        // The accounts the call used up; taking one again means the
        // rotation has no other.
        let mut tried: Vec<CredentialId> = Vec::new();
        let mut last_moment = None;
        loop {
            let used_up = |resets_at| refused(ImageRefusal::UsedUp { vendor: vendor.clone(), resets_at });
            // Another account is sent to only while the entry may still
            // make images.
            if !tried.is_empty()
                && current(services, self.shard.user(), &entry.id).await.map_err(failed)?.is_none()
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
            let Some(maker) = provider.images() else {
                return Err(refused(ImageRefusal::NoProvider));
            };
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
            provider: self.maker.entry.label.clone(),
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
        let entry = &self.maker.entry;
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
            .make(self.shard.http(), self.request.clone())
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
