//! The media of an entry lent to another server
//! (`crates/provider-relay/src/wire/media.rs`).
//!
//! A borrower's call runs on the shard of the key's maker as a call of theirs
//! does, on the entry's key or its accounts in turn (`super::media`), and is
//! answered by the maker's own methods ([`serve`]), with no usage ledger row:
//! the usage is the borrower's. What crosses back is what the types let cross
//! (`demi_provider_common::media::channel`); here a job's id at the vendor is
//! replaced by one of this server's, and a sign-in that could not be used
//! crosses as a refused key.
//!
//! A call runs in a task of the shard until its last reply, its key's revoke
//! or the borrower's going, which drops its replies. A video's job left
//! unfinished is then cancelled at the vendor, for a vendor that cancels one,
//! as a call of the key's maker is.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use demi_plugin_interface::MediaRefusal;
use demi_provider_common::media::channel::{Call, Reply, Roles, no_facts, serve};
use demi_provider_common::media::{MediaFailure, unreadable};
use demi_provider_common::media_catalog::Catalog;
use demi_provider_common::videos::{JobRef, JobStep};
use demi_provider_common::{ErrorCode, ProviderHttp};
use demi_provider_registry::registry::MakerOf;
use demi_shared_types::{MediaKind, VendorTrace};
use futures_util::stream::{BoxStream, StreamExt as _};
use tokio::sync::mpsc;

use super::Shard;
use super::media::{Route, Unmade, model_of, on_accounts, route};
use crate::provider_sharing::Borrowing;

/// Why a lent call was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unlent {
    /// The lent entry makes none of the call's kind now: its switch is off.
    MakesNone,
    /// The key was revoked as the call came.
    Revoked,
}

/// `call`'s replies from `maker`, which they own.
fn served(maker: MakerOf, http: ProviderHttp, catalog: Arc<Catalog>, call: Call, every: Duration) -> BoxStream<'static, Reply> {
    async_stream::stream! {
        let roles = Roles { media: maker.media(), images: maker.images(), videos: maker.videos(), audio: maker.audio() };
        let mut replies = serve(roles, &http, &catalog, call, every);
        while let Some(reply) = replies.next().await {
            yield reply;
        }
    }
    .boxed()
}

/// The failure a borrower is told of `unmade`.
fn told(unmade: Unmade) -> MediaFailure {
    let trace = VendorTrace::default();
    match unmade {
        Unmade::Vendor { failure, .. } => failure,
        Unmade::Refusal(MediaRefusal::UsedUp { .. }) => MediaFailure::UsageLimit { retry_at: None, trace },
        Unmade::Refusal(_) => MediaFailure::Unanswered { cause: String::new(), code: None, trace },
    }
}

/// `failure` as it may cross: a sign-in crosses as a refused key.
fn crossing(failure: MediaFailure) -> MediaFailure {
    match failure {
        MediaFailure::SignIn(failure) => MediaFailure::KeyRefused {
            message: String::new(),
            code: failure.failure().code.unwrap_or(ErrorCode::AuthExpired),
            trace: VendorTrace::default(),
        },
        failure => failure,
    }
}

/// A video's job a lent call asked for, which its end cancels unless the job
/// ended first.
struct Asked {
    job: String,
    maker: MakerOf,
}

impl Shard {
    /// Takes the borrower's `call` of the lent entry's maker of `kind`, and
    /// answers the receiver of its replies.
    pub async fn lent_media(
        self: Rc<Self>,
        borrowing: Borrowing,
        kind: MediaKind,
        call: Call,
    ) -> Result<mpsc::UnboundedReceiver<Reply>, Unlent> {
        let services = self.services();
        let entry = borrowing.entry.clone();
        let makes = services.provider_registry.media_shapes(&entry).makes(kind) && entry.generates(kind);
        let found = match services.control.preferences(self.user().clone()).await {
            Ok(preferences) => route(services, entry.clone(), &preferences, kind).await.map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        let (sender, receiver) = mpsc::unbounded_channel();
        let route = match (found, makes) {
            (Ok(Some(route)), true) => route,
            // A kind it makes none of lists no models, rather than failing
            // to list.
            (Ok(_), _) if matches!(call, Call::Facts) => {
                let _ = sender.send(Reply::Facts(Ok(no_facts(entry.label.clone()))));
                return Ok(receiver);
            }
            (Ok(_), _) => return Err(Unlent::MakesNone),
            (Err(error), _) => {
                tracing::warn!(event = "relay.media_unrouted", key = %borrowing.key.id, %error, "a lent media call found no route");
                let failure = MediaFailure::Unanswered { cause: String::new(), code: None, trace: VendorTrace::default() };
                let _ = sender.send(call.failed(failure));
                return Ok(receiver);
            }
        };
        let key = borrowing.key.id.clone();
        let revoked = services.lent.open(&key);
        // A revoke, or the entry's deletion, between the key's check and
        // `open` found no call to end: the key is checked again now that
        // one would be found.
        let held = services.lent_keys(&entry.id).await;
        if !held.is_ok_and(|keys| keys.contains(&key)) {
            services.lent.ended(&key, &revoked);
            return Err(Unlent::Revoked);
        }
        let shard = self.clone();
        self.tasks().spawn_local(async move {
            shard.lend_call(&borrowing, route, call, sender, &revoked).await;
            shard.services().lent.ended(&key, &revoked);
        });
        Ok(receiver)
    }

    /// Sends `call`'s replies to `sender` until the last, the borrower's
    /// going or `revoked`.
    async fn lend_call(
        &self,
        borrowing: &Borrowing,
        route: Route,
        call: Call,
        sender: mpsc::UnboundedSender<Reply>,
        revoked: &tokio_util::sync::CancellationToken,
    ) {
        let key = &borrowing.key.id;
        let services = self.services();
        let catalog = services.media.current();
        let every = services.media_tuning.video_reads_every;
        let http = route.http(services, self.http());
        let model = call.model().map(str::to_owned);
        let started = async {
            let Some(model) = &model else {
                let mut replies = served(route.maker.clone(), http.clone(), catalog.clone(), call.clone(), every);
                let first = replies.next().await;
                return Ok((first, replies, route.maker.clone()));
            };
            on_accounts(self, &route, &model_of(model), false, async |maker, _, _| {
                let mut replies = served(maker.clone(), http.clone(), catalog.clone(), call.clone(), every);
                let first = replies.next().await;
                match first.as_ref().and_then(Reply::failure) {
                    // Another account is tried.
                    Some(MediaFailure::UsageLimit { .. }) => {
                        let failure = first.and_then(|reply| reply.failure().cloned()).expect("a failure was read");
                        Err(Unmade::Vendor { failure, label: None })
                    }
                    _ => Ok((first, replies, maker.clone())),
                }
            })
            .await
        };
        let (first, mut rest, maker) = tokio::select! {
            started = started => match started {
                Ok(started) => started,
                Err(unmade) => {
                    let failure = told(unmade);
                    tracing::info!(event = "relay.media_ended", key, outcome = failure.outcome(), "a lent media call ended");
                    let _ = sender.send(call.failed(failure));
                    return;
                }
            },
            () = revoked.cancelled() => return,
        };
        let mut asked: Option<Asked> = None;
        let mut next = first;
        loop {
            let Some(reply) = next else {
                let _ = sender.send(call.failed(unreadable("its answer ended before its last reply".to_owned())));
                break;
            };
            let last = reply.is_last();
            let reply = match reply {
                Reply::Video(Ok(JobStep::Asked(job))) => {
                    asked = Some(Asked { job, maker: maker.clone() });
                    Reply::Video(Ok(JobStep::Asked(URL_SAFE_NO_PAD.encode(rand::random::<[u8; 12]>()))))
                }
                reply => reply,
            };
            if let Some(failure) = reply.failure() {
                tracing::info!(event = "relay.media_failed", key, outcome = failure.outcome(), "a lent media call failed");
            }
            if sender.send(reply.map_failure(crossing)).is_err() {
                break;
            }
            if last {
                asked = None;
                tracing::info!(event = "relay.media_ended", key, "a lent media call ended");
                break;
            }
            next = tokio::select! {
                reply = rest.next() => reply,
                () = sender.closed() => break,
                () = revoked.cancelled() => break,
            };
        }
        drop(rest);
        if let (Some(asked), Some(model)) = (asked, model) {
            cancel(asked, http, catalog, model, key.clone());
        }
    }
}

/// Cancels `asked`'s job at the vendor, for a vendor that cancels one;
/// nothing awaits it.
fn cancel(asked: Asked, http: ProviderHttp, catalog: Arc<Catalog>, model: String, key: String) {
    tokio::spawn(async move {
        let job = JobRef { id: &asked.job, model: &model, catalog: &catalog };
        let Some(cancel) = asked.maker.videos().and_then(|videos| videos.cancel(&http, job)) else {
            return;
        };
        if let Err(failure) = cancel.await.result {
            tracing::info!(event = "relay.media_cancel_failed", key, code = failure.outcome(), "a lent video's job was not cancelled at the vendor");
        }
    });
}
