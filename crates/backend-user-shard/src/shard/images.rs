//! The Providers service of the plugins' port (`images.md`, `plugins.md`
//! § The contract): the image models of the user's entries, and one image
//! made through an entry's account. The service chooses the entry and,
//! under the entry's rotation, the account, as a new agent takes one at its
//! first request; an account used up moves the call to another at once, as
//! a turn's failure would. The provider makes the image, renewing its
//! account's sign-in, and each call is a usage ledger row with the purpose
//! `image`. What the agent reads of it, the plugin words.

use std::sync::Arc;

use demi_backend_database::usage::{UsageRow, merge_windows};
use demi_backend_providers::llm::rotation::{Choice, USED_UP_FOR_MS};
use demi_backend_providers::vault::entries::{EntryCredential, ProviderEntry};
use demi_plugin_interface::{ImageAsk, ImageModel, ImageRefusal, MadeImage, PortFailure, PortRefusal};
use demi_provider_common::Provider;
use demi_provider_common::credentials::AuthReason;
use demi_provider_common::images::{Aspect, ImageFailure, ImageMaker, ImageRequest, InputImage};
use demi_host_interface::{JobCaller, PortError};
use demi_shared_types::{Model, Rotation, Timestamp, TurnId};
use demi_web_api_protocol::ids::{ConversationId, CredentialId};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::Shard;
use crate::conversation::product::entries;
use demi_backend_host_access::root_of;

/// An entry that makes images, with the provider of its active account.
struct Maker {
    entry: ProviderEntry,
    provider: Arc<dyn Provider>,
}

impl Maker {
    fn images(&self) -> &dyn ImageMaker {
        self.provider.images().expect("a maker's provider makes images")
    }
}

fn refused(refusal: ImageRefusal) -> PortFailure {
    PortFailure::Refused(PortRefusal::Image { refusal })
}

fn failed(error: impl ToString) -> PortFailure {
    refused(ImageRefusal::Failed { message: error.to_string() })
}

/// The user's entries whose providers make images, in the order of
/// Settings › Providers: each subscription entry the user has on, with an
/// account, whose provider makes images; `on` those whose Image Generation
/// is on, `off` the others (`providers.md` § Provider management).
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
}

async fn makers(shard: &Shard) -> Result<Makers, PortFailure> {
    let services = shard.services();
    let preferences = services.control.preferences(shard.user().clone()).await.map_err(failed)?;
    let mut makers = Makers { on: Vec::new(), off: Vec::new() };
    for entry in entries(shard).await.map_err(failed)? {
        let signed_in = matches!(entry.credential, EntryCredential::Subscription { active: Some(_), .. });
        if !signed_in || preferences.hides_provider(&entry.id) {
            continue;
        }
        let provider = services.assembly.provider_for(&entry).await.map_err(failed)?;
        if provider.images().is_none() {
            continue;
        }
        let on = entry.image_generation;
        let maker = Maker { entry, provider };
        if on { makers.on.push(maker) } else { makers.off.push(maker) }
    }
    Ok(makers)
}

/// The image models of the user's entries whose Image Generation is on,
/// in the order of Settings › Providers, each entry's newest first by the
/// vendor's release date, the first entry's default model the default
/// (`images.md` § Commands); when every entry that could make images is
/// off, the refusal that says so.
pub(crate) async fn image_models(shard: &Shard) -> Result<Vec<ImageModel>, PortFailure> {
    let makers = makers(shard).await?;
    if makers.on.is_empty() && !makers.off.is_empty() {
        return Err(refused(makers.none()));
    }
    let mut models = Vec::new();
    for (index, maker) in makers.on.iter().enumerate() {
        let images = maker.images();
        let mut offered = images.models().to_vec();
        // ISO dates sort as text; a sort that keeps the provider's order
        // for one date.
        offered.sort_by(|a, b| b.released.cmp(a.released));
        for model in offered {
            models.push(ImageModel {
                id: model.id.to_owned(),
                provider: maker.entry.label.clone(),
                provider_id: maker.entry.id.as_str().to_owned(),
                vendor: images.vendor().to_owned(),
                released: model.released.to_owned(),
                description: model.description.to_owned(),
                takes: model.takes.to_owned(),
                default: index == 0 && model.id == images.default_model(),
                confirmed: images.confirms_model(),
            });
        }
    }
    Ok(models)
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
    let mut makers = makers(shard).await?;
    let makes = |maker: &Maker, model: &str| maker.images().models().iter().any(|offered| offered.id == model);
    let (maker, model) = match &ask.model {
        Some(model) => match makers.on.iter().position(|maker| makes(maker, model)) {
            Some(index) => (makers.on.swap_remove(index), model.clone()),
            None => {
                let refusal = match makers.off.iter().find(|maker| makes(maker, model)) {
                    Some(maker) => ImageRefusal::Off {
                        model: model.clone(),
                        provider: maker.entry.label.clone(),
                    },
                    None => ImageRefusal::UnknownModel { model: model.clone() },
                };
                return Err(refused(refusal));
            }
        },
        None => {
            if makers.on.is_empty() {
                return Err(refused(makers.none()));
            }
            let maker = makers.on.swap_remove(0);
            let model = maker.images().default_model().to_owned();
            (maker, model)
        }
    };
    let request = ImageRequest {
        model,
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
    let user = shard.user();
    let manages = shard.services().vault.owner_for(user).await.map_err(failed)? == *user;
    let making = Making {
        shard,
        manages,
        turn: caller.and_then(|caller| caller.turn.clone()),
        entry: &maker.entry,
        request: &request,
        vendor: maker.images().vendor(),
        agent,
        conversation: conversation.clone(),
        model: Model {
            id: request.model.clone(),
            name: request.model.clone(),
            context_window: 0,
            output_limit: None,
            thinking: Vec::new(),
            accepted_extensions: None,
        },
    };
    tokio::select! {
        made = making.run() => made,
        () = cancel.cancelled() => Err(PortError::Ended("the call was stopped".into()).into()),
    }
}

/// One image made on an entry: the account it takes under the entry's
/// rotation, and another at once when that one is used up.
struct Making<'a> {
    shard: &'a Shard,
    entry: &'a ProviderEntry,
    request: &'a ImageRequest,
    vendor: &'static str,
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
        let services = self.shard.services();
        let assembly = &services.assembly;
        let rotation = self.entry.rotation();
        let vendor = self.vendor.to_owned();
        // The accounts the call used up; taking one again means the
        // rotation has no other.
        let mut tried: Vec<CredentialId> = Vec::new();
        let mut last_moment = None;
        loop {
            let used_up = |resets_at| refused(ImageRefusal::UsedUp { vendor: vendor.clone(), resets_at });
            let (account, running) = match rotation {
                Rotation::Manual => {
                    let active = self.entry.active().cloned();
                    (active.ok_or_else(|| refused(ImageRefusal::NoProvider))?, None)
                }
                Rotation::InOrder | Rotation::Balanced => {
                    let (choice, _) = assembly.take_account(self.entry, &self.model).await.map_err(failed)?;
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
                .for_account(self.entry, &account)
                .await
                .map_err(failed)?
                .ok_or_else(|| refused(ImageRefusal::NoProvider))?;
            // The account is named only to whoever manages the entry: on a
            // shared instance it is the administrator's (`providers.md`
            // § Scope).
            let label = if self.manages {
                let record = services.vault.account(self.entry.id.clone(), account.clone()).await.ok().flatten();
                Some(record.map_or_else(|| account.to_string(), |record| record.label))
            } else {
                None
            };
            let Some(maker) = provider.images() else {
                return Err(refused(ImageRefusal::NoProvider));
            };
            let made = self.call(maker, &account).await;
            // The call's account counts no longer: it made its one image.
            drop(running);
            let failure = match made {
                Ok(image) => {
                    return Ok(MadeImage {
                        bytes: image.bytes.into(),
                        model: self.request.model.clone(),
                        provider: self.entry.label.clone(),
                        vendor,
                        account: label,
                        confirmed: maker.confirms_model(),
                    });
                }
                Err(failure) => failure,
            };
            let ImageFailure::UsageLimit { retry_at } = failure else {
                return Err(refused(vendor_refusal(failure, vendor, label)));
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
            assembly.rotation().mark_used_up(&self.entry.id, &account, until);
            if rotation == Rotation::Manual {
                return Err(used_up(moment));
            }
            tried.push(account);
            last_moment = moment;
        }
    }

    /// One call to the vendor on `account`, and its ledger row; a call
    /// dropped before its answer is a cancelled row.
    async fn call(
        &self,
        maker: &dyn ImageMaker,
        account: &CredentialId,
    ) -> Result<demi_provider_common::images::MadeImage, ImageFailure> {
        let services = self.shard.services();
        let request_id = uuid::Uuid::new_v4().to_string();
        let span = tracing::info_span!(
            "image_call",
            request = %request_id,
            agent = self.agent,
            account = %account,
            provider = %self.entry.id,
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
                provider: self.entry.id.clone(),
                account: Some(account.clone()),
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
/// image, other than a used-up account.
fn vendor_refusal(failure: ImageFailure, vendor: String, account: Option<String>) -> ImageRefusal {
    match failure {
        ImageFailure::SignIn(failure) if matches!(failure.reason, AuthReason::Store(_)) => {
            ImageRefusal::Failed { message: failure.message() }
        }
        ImageFailure::SignIn(_) => ImageRefusal::Expired { vendor, account },
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
