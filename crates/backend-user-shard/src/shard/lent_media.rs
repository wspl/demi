//! The media of an entry lent to another server
//! (`crates/provider-relay/src/wire/media.rs`).
//!
//! A borrower's call runs on the shard of the key's maker as a call of theirs
//! does, on the entry's key or its accounts in turn (`super::media`), and is
//! answered by the maker's own methods ([`serve`]), with no usage ledger row:
//! the usage is the borrower's. What crosses back is what the types let cross
//! (`demi_provider_common::media::channel`), and a sign-in that could not be
//! used crosses as a refused key.
//!
//! A video's job crosses as a handle of this server's ([`LentJobs`]): the
//! borrower's reads and cancel of the job name it, and are served by the
//! maker and account that took the job, whatever the entry's rotation says
//! by then. The handles live in memory, so a restart of this server loses
//! the jobs it served, and the borrower is told the job is lost. A handle
//! outlives its job's last read for as long as the borrower may read it.
//!
//! A call runs in a task of the shard until its reply, its key's revoke or
//! the borrower's going, which drops it. Only the borrower's cancel cancels a
//! job at the vendor; its going cancels only a job whose handle it could not
//! be sent.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use demi_plugin_interface::MediaRefusal;
use demi_provider_common::media::channel::{Call, Reply, Roles, no_facts, serve};
use demi_provider_common::media::MediaFailure;
use demi_provider_common::media_catalog::Catalog;
use demi_provider_common::videos::JobState;
use demi_provider_common::{ErrorCode, ProviderHttp};
use demi_provider_registry::registry::MakerOf;
use demi_shared_types::{MediaKind, VendorTrace};
use tokio::sync::oneshot;
use tokio::time::Instant;

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

/// The videos' jobs borrowers asked for, by the handle each was sent.
#[derive(Default)]
pub(crate) struct LentJobs(RefCell<HashMap<String, LentJob>>);

/// A job a borrower asked for: who took it, and until when its handle is
/// kept.
#[derive(Clone)]
struct LentJob {
    /// The share key it was asked with, whose calls alone name it.
    key: String,
    maker: MakerOf,
    http: ProviderHttp,
    /// The vendor's id of the job.
    job: String,
    model: String,
    kept_until: Instant,
}

impl LentJobs {
    /// Keeps `job` and answers its new handle; drops the handles kept past
    /// their time.
    fn keep(&self, job: LentJob) -> String {
        let handle = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 12]>());
        let mut jobs = self.0.borrow_mut();
        let now = Instant::now();
        jobs.retain(|_, kept| kept.kept_until > now);
        jobs.insert(handle.clone(), job);
        handle
    }

    /// The job `handle` names, asked with `key`; none for a handle this
    /// server does not keep.
    fn of(&self, key: &str, handle: &str) -> Option<LentJob> {
        let jobs = self.0.borrow();
        jobs.get(handle).filter(|job| job.key == key && job.kept_until > Instant::now()).cloned()
    }

    fn forget(&self, handle: &str) {
        self.0.borrow_mut().remove(handle);
    }
}

/// `call`'s reply from `maker`.
async fn served(maker: &MakerOf, http: &ProviderHttp, catalog: &Catalog, call: Call) -> Reply {
    let roles = Roles { media: maker.media(), images: maker.images(), videos: maker.videos(), audio: maker.audio() };
    serve(roles, http, catalog, call).await
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
            code: failure.failure().code.unwrap_or(ErrorCode::AuthExpired),
            trace: VendorTrace::default(),
        },
        failure => failure,
    }
}

impl Shard {
    /// Takes the borrower's `call` of the lent entry's maker of `kind`, and
    /// answers the receiver of its reply.
    pub async fn lent_media(
        self: Rc<Self>,
        borrowing: Borrowing,
        kind: MediaKind,
        call: Call,
    ) -> Result<oneshot::Receiver<Reply>, Unlent> {
        let services = self.services();
        let (sender, receiver) = oneshot::channel();
        let key = borrowing.key.id.clone();
        // A job's reads and cancel go to the maker that took it.
        if let Call::VideoState { job: handle, .. } | Call::VideoCancel { job: handle, .. } = &call {
            let Some(lent) = self.lent_jobs.of(&key, handle) else {
                let reply = match call {
                    Call::VideoState { .. } => Reply::VideoState(Ok(JobState::Lost)),
                    _ => Reply::VideoCancelled(Ok(())),
                };
                let _ = sender.send(reply);
                return Ok(receiver);
            };
            let handle = handle.clone();
            let shard = self.clone();
            let revoked = services.lent.open(&key);
            self.tasks().spawn_local(async move {
                shard.lend_job_call(lent, &handle, call, sender, &revoked).await;
                shard.services().lent.ended(&key, &revoked);
            });
            return Ok(receiver);
        }
        let entry = borrowing.entry.clone();
        let makes = services.provider_registry.media_shapes(&entry).makes(kind) && entry.generates(kind);
        let found = match self.store().preferences().await {
            Ok(preferences) => route(services, entry.clone(), &preferences, kind).await.map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
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
                tracing::warn!(event = "relay.media_unrouted", key = %key, %error, "a lent media call found no route");
                let failure = MediaFailure::Unanswered { cause: String::new(), code: None, trace: VendorTrace::default() };
                let _ = sender.send(call.failed(failure));
                return Ok(receiver);
            }
        };
        let revoked = services.lent.open(&key);
        // A revoke, or the entry's deletion, between the key's check and
        // `open` found no call to end: the key is checked again now that
        // one would be found.
        let held = services.lent_keys(&entry).await;
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

    /// Sends `call`'s reply to `sender`, unless the borrower went or the key
    /// was `revoked` first.
    async fn lend_call(
        &self,
        borrowing: &Borrowing,
        route: Route,
        call: Call,
        mut sender: oneshot::Sender<Reply>,
        revoked: &tokio_util::sync::CancellationToken,
    ) {
        let key = &borrowing.key.id;
        let services = self.services();
        let catalog = services.media.current();
        let http = route.http(services, self.http());
        let answered = async {
            let Some(model) = call.model().map(str::to_owned) else {
                let reply = served(&route.maker, &http, &catalog, call.clone()).await;
                return Ok((reply, route.maker.clone(), None));
            };
            on_accounts(self, &route, &model_of(&model), false, async |maker, _, _| {
                let reply = served(maker, &http, &catalog, call.clone()).await;
                match reply.failure() {
                    // Another account is tried.
                    Some(MediaFailure::UsageLimit { .. }) => {
                        let failure = reply.failure().cloned().expect("a failure was read");
                        Err(Unmade::Vendor { failure, label: None })
                    }
                    _ => Ok((reply, maker.clone(), Some(model.clone()))),
                }
            })
            .await
        };
        let (reply, maker, model) = tokio::select! {
            answered = answered => match answered {
                Ok(answered) => answered,
                Err(unmade) => (call.failed(told(unmade)), route.maker.clone(), None),
            },
            () = sender.closed() => return,
            () = revoked.cancelled() => return,
        };
        // The vendor's id stays here: the borrower is sent a handle.
        let (reply, handle) = match (reply, model) {
            (Reply::VideoAsked(Ok(job)), Some(model)) => {
                let kept_until = Instant::now() + services.media_tuning.job_bound;
                let lent = LentJob { key: key.clone(), maker, http, job, model, kept_until };
                let handle = self.lent_jobs.keep(lent);
                (Reply::VideoAsked(Ok(handle.clone())), Some(handle))
            }
            (reply, _) => (reply, None),
        };
        match reply.failure() {
            Some(failure) => tracing::info!(event = "relay.media_failed", key, outcome = failure.outcome(), "a lent media call failed"),
            None => tracing::info!(event = "relay.media_ended", key, "a lent media call ended"),
        }
        // A job whose handle the borrower could not be sent is one it can
        // neither read nor cancel.
        if sender.send(reply.map_failure(crossing)).is_err()
            && let Some(handle) = handle
            && let Some(lent) = self.lent_jobs.of(key, &handle)
        {
            self.lent_jobs.forget(&handle);
            let call = Call::VideoCancel { job: lent.job.clone(), model: lent.model.clone() };
            let catalog = services.media.current();
            self.tasks().spawn_local(async move {
                if let Some(failure) = served(&lent.maker, &lent.http, &catalog, call).await.failure() {
                    tracing::info!(event = "relay.media_cancel_failed", key = lent.key, code = failure.outcome(), "a lent video's job was not cancelled at the vendor");
                }
            });
        }
    }

    /// Sends the reply of `call`, a read or the cancel of the job `lent`,
    /// whose handle is `handle`, to `sender`.
    async fn lend_job_call(
        &self,
        lent: LentJob,
        handle: &str,
        call: Call,
        mut sender: oneshot::Sender<Reply>,
        revoked: &tokio_util::sync::CancellationToken,
    ) {
        let cancels = matches!(call, Call::VideoCancel { .. });
        let call = match call {
            Call::VideoCancel { .. } => Call::VideoCancel { job: lent.job.clone(), model: lent.model.clone() },
            _ => Call::VideoState { job: lent.job.clone(), model: lent.model.clone() },
        };
        let catalog = self.services().media.current();
        let reply = served(&lent.maker, &lent.http, &catalog, call);
        let reply = tokio::select! {
            reply = reply => reply,
            () = sender.closed() => return,
            () = revoked.cancelled() => return,
        };
        if cancels {
            self.lent_jobs.forget(handle);
        }
        if let Some(failure) = reply.failure() {
            tracing::info!(event = "relay.media_failed", key = lent.key, outcome = failure.outcome(), "a lent video's job call failed");
        }
        let _ = sender.send(reply.map_failure(crossing));
    }
}
