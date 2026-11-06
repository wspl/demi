//! The media jobs vendors took, followed to their ends whatever happens to
//! the calls that asked for them (`demi_backend_database::media_jobs`).
//!
//! ```text
//! make_video ---- ask ----> vendor          (the call)
//!     | row: running                            |
//!     v                                          |
//! follower: read every few seconds <-------------+   (the shard's task)
//!     | the end: the usage row, then the row ended,
//!     |          the file in the user's blobs
//!     v
//! collector: the steps the call is told, then the row's end,
//!            collected and deleted                   (the call)
//! ```
//!
//! - A follower belongs to the shard, not to a call: a call that goes, or a
//!   connection that drops, leaves it reading. One job has one follower,
//!   which whoever finds the job unfollowed starts, before any wait.
//! - A stop of the call that asked, unless the shard is closing or the
//!   runner sends the call again, leaves the job to nobody: a video's is
//!   cancelled at a vendor that cancels one, an image's or a sound's runs to
//!   its end, and either records its cost and leaves no row.
//! - A shard that closes stops its followers where they are, and their rows
//!   stay running: the next start follows them again, from the vendor's id,
//!   on the entry and account the job was asked on.
//! - The wait is bounded from the job's ask: a job still running then ends,
//!   while the vendor goes on with it, as one the call no longer waits for.
//!   A vendor that does not answer a read is read again until then.
//! - A call the runner sends again after a restart finds its job by its id
//!   and collects it as the first call would have.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use demi_backend_database::media_jobs::{JobEnd, JobOfCall, Made, MediaJob, MediaJobStatus};
use demi_backend_database::usage::{UsageRow, merge_windows};
use demi_backend_device_access::DeviceShard;
use demi_device_interface::PortError;
use demi_plugin_host::{MediaCall, VideoJob};
use demi_plugin_interface::{MadeAudio, MadeImage, MadeVideo, MediaRefusal, PcmShape, PortFailure, PortRefusal, VideoStep};
use demi_provider_common::ProviderHttp;
use demi_provider_common::media::jobs::{JobRef, JobState};
use demi_provider_common::media::{MediaAnswer, MediaFailure, unreadable};
use demi_provider_common::media_catalog::Catalog;
use demi_provider_common::videos::{Done, DoneVideo};
use demi_provider_registry::registry::MakerOf;
use demi_provider_registry::store::StoreError;
use demi_shared_types::{B64Bytes, MediaKind, QuotaWindow, mp4};
use futures_util::StreamExt as _;
use futures_util::stream::LocalBoxStream;
use sha2::{Digest as _, Sha256};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::Shard;
use super::media::vendor_refusal;

/// The jobs the shard follows, by their calls.
#[derive(Default)]
pub(crate) struct MediaJobs(RefCell<HashMap<JobOfCall, Rc<Followed>>>);

/// A job a follower reads.
pub(crate) struct Followed {
    standing: watch::Sender<Standing>,
    /// Cancelled by a stop of the call that asked for it, after which no
    /// call collects the job.
    stop: CancellationToken,
}

impl Followed {
    /// Leaves the job to nobody: a video's is cancelled at a vendor that
    /// cancels one, and the job's end leaves no row.
    fn abandon(&self) {
        self.stop.cancel();
    }

    fn abandoned(&self) -> bool {
        self.stop.is_cancelled()
    }
}

/// Where a followed job stands, as its call is told.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Standing {
    Queued,
    Running { progress: Option<u8> },
    /// Its row says how.
    Ended,
}

/// What reads a job: its maker, on the account it was asked on, and the
/// client of the maker's entry.
#[derive(Clone)]
pub(crate) struct Reader {
    pub(crate) maker: MakerOf,
    pub(crate) http: ProviderHttp,
}

fn refused(refusal: MediaRefusal) -> PortFailure {
    PortFailure::Refused(PortRefusal::Media { refusal })
}

impl Shard {
    /// Follows `job` until it ends, unless it is followed already, and
    /// answers its follow. A job without its `reader`, one an earlier start
    /// kept, is read on its entry and account as they are now, unless it
    /// ended meanwhile.
    pub(crate) fn follow_media_job(&self, job: MediaJob, reader: Option<Reader>) -> Rc<Followed> {
        let mut jobs = self.media_jobs.0.borrow_mut();
        if let Some(followed) = jobs.get(&job.of) {
            return followed.clone();
        }
        let followed = Rc::new(Followed { standing: watch::Sender::new(Standing::Queued), stop: CancellationToken::new() });
        jobs.insert(job.of.clone(), followed.clone());
        drop(jobs);
        let shard = self.this();
        let follow = followed.clone();
        let span = tracing::info_span!(
            "media_job",
            kind = job.kind.plural(),
            job = %job.job,
            conversation = %job.usage.conversation,
            provider = %job.usage.provider,
            model = %job.usage.model,
        );
        self.tasks().spawn_local(
            async move {
                let of = job.of.clone();
                let closing = shard.closed();
                tokio::select! {
                    () = shard.follow_kept(job, reader, &follow) => {}
                    // The row stays running for the next start.
                    () = closing => {}
                }
                shard.media_jobs.0.borrow_mut().remove(&of);
                follow.standing.send_replace(Standing::Ended);
            }
            .instrument(span),
        );
        followed
    }

    /// Follows each of the user's running jobs, as the shard starts.
    pub(crate) async fn follow_media_jobs(self: Rc<Self>) {
        let jobs = match self.services().control.running_media_jobs(self.user().clone()).await {
            Ok(jobs) => jobs,
            Err(error) => {
                tracing::warn!(event = "media_job.list_failed", %error, "the running media jobs were not read");
                return;
            }
        };
        for job in jobs {
            self.follow_media_job(job, None);
        }
    }

    /// Follows `job` with `reader`, or, for a job an earlier start kept,
    /// on its entry and account as they are now; a job they no longer read
    /// ends failed.
    async fn follow_kept(&self, job: MediaJob, reader: Option<Reader>, followed: &Followed) {
        let reader = match reader {
            Some(reader) => reader,
            None => {
                // A job whose follower ended between its collector's read
                // and its start is not read again.
                if !matches!(self.services().control.media_job(job.of.clone()).await, Ok(Some(MediaJob { status: MediaJobStatus::Running, .. }))) {
                    return;
                }
                match self.reader_of(&job).await {
                    Ok(reader) => reader,
                    Err(message) => {
                        let refusal = MediaRefusal::Failed { message };
                        let mut row = job.usage.clone();
                        row.outcome = "failed".to_owned();
                        self.end_job(&job, row, MediaJobStatus::Failed { refusal }, followed).await;
                        return;
                    }
                }
            }
        };
        self.follow(job, reader, followed).await;
    }

    /// The reader of `job` built anew: its entry, with the account it was
    /// asked on, as the user's vault has them now.
    async fn reader_of(&self, job: &MediaJob) -> Result<Reader, String> {
        let services = self.services();
        let gone = || "the provider that took the job is no longer there".to_owned();
        let entry = services
            .vault
            .entry(job.usage.provider.clone())
            .await
            .map_err(|error| StoreError::new(error).to_string())?
            .ok_or_else(gone)?;
        let registry = &services.provider_registry;
        let provider = match &job.usage.account {
            Some(account) => registry.for_account(&entry, account).await.map_err(|error| error.to_string())?.ok_or_else(gone)?,
            None => registry.provider_for(&entry).await.map_err(|error| error.to_string())?,
        };
        let maker = registry.maker(&entry, &provider, job.kind).ok_or_else(gone)?;
        Ok(Reader { maker, http: registry.http_for(&entry, self.http().clone()) })
    }

    /// Reads `job` until it ends, then writes its usage row and its end.
    async fn follow(&self, job: MediaJob, reader: Reader, followed: &Followed) {
        let services = self.services();
        let tuning = services.media_tuning;
        let mut row = job.usage.clone();
        let follow = &job.follow;
        // A failure of the vendor's ends the job with its outcome.
        let failed = |failure: MediaFailure| {
            let outcome = failure.outcome();
            let refusal = vendor_refusal(failure, follow.vendor.clone(), follow.label.clone(), follow.manages);
            (MediaJobStatus::Failed { refusal }, outcome)
        };
        let ended = |refusal: MediaRefusal| (MediaJobStatus::Failed { refusal }, "failed".to_owned());
        let elapsed = u64::try_from(services.clock.now().as_millisecond() - job.asked_at.as_millisecond()).unwrap_or(0);
        let deadline = tokio::time::Instant::now() + tuning.job_bound.saturating_sub(Duration::from_millis(elapsed));
        let mut first = true;
        // A video's job is cancelled once its call is stopped; an image's
        // or a sound's, which no vendor cancels, is read to its end.
        let cancels = job.kind == MediaKind::Video;
        let (status, outcome) = loop {
            if !first {
                let paused = tokio::select! {
                    () = tokio::time::sleep(tuning.job_reads_every) => true,
                    () = followed.stop.cancelled(), if cancels => false,
                };
                if !paused {
                    tracing::info!(event = "media_job.stopped", "a media job was stopped by its call");
                    self.cancel_job(&job, &reader).await;
                    break (MediaJobStatus::Cancelled, "cancelled".to_owned());
                }
            }
            first = false;
            if tokio::time::Instant::now() >= deadline {
                // The job goes on at the vendor, which the call's first
                // line named.
                break ended(MediaRefusal::StillRunning {
                    vendor: follow.vendor.clone(),
                    job: job.job.clone(),
                    seconds: tuning.job_bound.as_secs(),
                });
            }
            let answer = read(&job, &reader, &services.media.current()).await;
            observe(&mut row, &answer.observed);
            let state = match answer.result {
                Ok(state) => state,
                // Read again until the bound.
                Err(MediaFailure::Unanswered { cause, .. }) => {
                    tracing::info!(event = "media_job.unanswered", cause, "a read of a media job was not answered");
                    continue;
                }
                Err(failure) => break failed(failure),
            };
            let standing = match state {
                JobState::Queued => Standing::Queued,
                JobState::Running { progress } => Standing::Running { progress },
                JobState::Done(finished) => match self.made(&job, &reader, &mut row, finished).await {
                    Ok(made) => break (made, "answered".to_owned()),
                    Err(failure) => break failed(failure),
                },
                JobState::Failed { reason, trace } => match job.kind {
                    MediaKind::Video => break ended(MediaRefusal::JobFailed { vendor: follow.vendor.clone(), reason, trace }),
                    // An image or a sound fails as its one request would.
                    _ => break failed(MediaFailure::Refused { reason, code: None, trace }),
                },
                JobState::Expired => break ended(MediaRefusal::JobExpired { vendor: follow.vendor.clone() }),
                JobState::Lost => break ended(MediaRefusal::JobLost { vendor: follow.vendor.clone() }),
            };
            followed.standing.send_if_modified(|told| {
                let changed = *told != standing;
                *told = standing;
                changed
            });
        };
        row.outcome = outcome;
        self.end_job(&job, row, status, followed).await;
    }

    /// Cancels `job` at the vendor, for a video's vendor that cancels one.
    async fn cancel_job(&self, job: &MediaJob, reader: &Reader) {
        let Some(videos) = reader.maker.videos().filter(|_| job.kind == MediaKind::Video) else {
            return;
        };
        let catalog = self.services().media.current();
        let id = JobRef { id: &job.job, model: &job.usage.model, catalog: &catalog };
        if let Some(cancel) = videos.cancel(&reader.http, id)
            && let Err(failure) = cancel.await.result
        {
            tracing::warn!(event = "media_job.cancel_failed", code = failure.outcome(), "a stopped media job was not cancelled at the vendor");
        }
    }

    /// What the finished job made, kept in the user's blobs, with what the
    /// call answers. What the vendor reported goes on the row at once, so a
    /// download that then fails still records what was paid.
    async fn made(&self, job: &MediaJob, reader: &Reader, row: &mut UsageRow, finished: Finished) -> Result<MediaJobStatus, MediaFailure> {
        let (bytes, made) = match (finished, job.follow.answer.clone()) {
            (Finished::Video(done), Made::Video { mut video }) => {
                row.cost = done.cost;
                row.usage = done.usage;
                let bytes = match done.video {
                    DoneVideo::Sent(bytes) => bytes,
                    DoneVideo::At(url) => {
                        let videos = reader.maker.videos().expect("a video's job is a video maker's");
                        let answer = videos.download(&reader.http, &url).await;
                        observe(row, &answer.observed);
                        answer.result?
                    }
                };
                // A file this does not read is the plugin's to refuse; its
                // row keeps no seconds.
                row.media_seconds = mp4::read(&bytes).ok().map(|facts| facts.seconds);
                row.media_digest = Some(hex::encode(Sha256::digest(&bytes)));
                video.cost = done.cost;
                (bytes, Made::Video { video })
            }
            (Finished::Image(image), Made::Image { image: answer }) => {
                row.cost = image.cost;
                row.usage = image.usage;
                row.vendor_id = image.vendor_id.or(row.vendor_id.take());
                (image.bytes, Made::Image { image: answer })
            }
            (Finished::Audio(audio), Made::Audio { audio: mut answer, of }) => {
                row.cost = audio.cost;
                row.usage = audio.usage;
                row.vendor_id = audio.vendor_id.or(row.vendor_id.take());
                // A file this does not read is the plugin's to refuse; its
                // row keeps no seconds.
                row.media_seconds = match audio.pcm {
                    Some(pcm) => Some(pcm.seconds(audio.bytes.len())),
                    None => demi_shared_types::audio::read(&audio.bytes).ok().map(|facts| facts.seconds),
                };
                answer.pcm = audio.pcm.map(|pcm| PcmShape { sample_rate: pcm.sample_rate(), channels: pcm.channels(), bits: pcm.bits() });
                answer.cost = audio.cost;
                (audio.bytes, Made::Audio { audio: answer, of })
            }
            _ => return Err(unreadable("its finished job made another kind of media".to_owned())),
        };
        let blob = DeviceShard::blobs(self).put(bytes).await.map_err(|error| MediaFailure::Unanswered {
            cause: format!("the file could not be kept: {error}"),
            code: None,
            trace: Default::default(),
        })?;
        Ok(MediaJobStatus::Made { blob, made })
    }

    /// Ends `job` as `status` says, with its usage `row` ended now; a job
    /// its call left to nobody leaves no row.
    async fn end_job(&self, job: &MediaJob, mut row: UsageRow, status: MediaJobStatus, followed: &Followed) {
        row.ended_at = self.services().clock.now();
        let outcome = row.outcome.clone();
        let end = JobEnd { of: job.of.clone(), status, usage: row, kept: !followed.abandoned() };
        match self.services().control.end_media_job(end).await {
            Ok(true) => tracing::info!(event = "media.ended", kind = job.kind.plural(), stage = "job", outcome, "a media job ended"),
            Ok(false) => tracing::info!(event = "media_job.ended_already", "a media job had ended already"),
            Err(error) => tracing::warn!(event = "media_job.end_failed", %error, "a media job's end was not kept"),
        }
    }

    /// The steps of the job of `of`, followed or kept, as its call is told
    /// them: each change of its state, then what it made or why it ended.
    /// Its end deletes the job's row and so lets its file go. Dropped, it
    /// stops only the call's waiting.
    pub(crate) fn collect(&self, of: JobOfCall) -> LocalBoxStream<'static, Result<Collected, PortFailure>> {
        let shard = self.this();
        async_stream::stream! {
            let control = shard.services().control.clone();
            let gone = || refused(MediaRefusal::Failed { message: "the server no longer keeps the job".to_owned() });
            let job = match control.media_job(of.clone()).await {
                Ok(Some(job)) => job,
                Ok(None) => {
                    yield Err(gone());
                    return;
                }
                Err(error) => {
                    yield Err(PortFailure::from(PortError::Ended(error.to_string())));
                    return;
                }
            };
            let followed = match job.status {
                MediaJobStatus::Running => Some(shard.follow_media_job(job, None)),
                _ => None,
            };
            if let Some(followed) = followed {
                let mut standing = followed.standing.subscribe();
                let mut told = None;
                loop {
                    let now = standing.borrow_and_update().clone();
                    if now == Standing::Ended {
                        break;
                    }
                    if told.as_ref() != Some(&now) {
                        told = Some(now.clone());
                        yield Ok(Collected::Step(now));
                    }
                    if standing.changed().await.is_err() {
                        break;
                    }
                }
            }
            let ended = match control.media_job(of.clone()).await {
                Ok(Some(job)) => job,
                Ok(None) => {
                    yield Err(gone());
                    return;
                }
                Err(error) => {
                    yield Err(PortFailure::from(PortError::Ended(error.to_string())));
                    return;
                }
            };
            let collected = match ended.status {
                // A shard that closes stops its followers, and the call
                // answers nothing: the end of its session stops it, which
                // the runner hears of as the session's end and sends the
                // call again after the restart.
                MediaJobStatus::Running => {
                    std::future::pending::<()>().await;
                    return;
                }
                MediaJobStatus::Made { blob, made } => match DeviceShard::blobs(&*shard).get(&blob).await {
                    Ok(Some(bytes)) => Ok(Collected::Made(with_bytes(made, bytes.into()))),
                    Ok(None) => Err(refused(MediaRefusal::Failed { message: "the file the job made is gone".to_owned() })),
                    // The row stays for a call that comes back.
                    Err(error) => {
                        yield Err(PortFailure::from(PortError::Ended(error.to_string())));
                        return;
                    }
                },
                MediaJobStatus::Failed { refusal } => Err(refused(refusal)),
                MediaJobStatus::Cancelled => Err(PortFailure::from(PortError::Ended("the call was stopped".into()))),
            };
            if let Err(error) = control.delete_media_job(of).await {
                tracing::warn!(event = "media_job.delete_failed", %error, "a collected media job was not deleted");
            }
            yield collected;
        }
        .boxed_local()
    }

    /// [`Shard::collect`] of a video's job, as the plugin reads its steps.
    pub(crate) fn collect_video(&self, of: JobOfCall) -> VideoJob {
        self.collect(of)
            .filter_map(|collected| {
                std::future::ready(match collected {
                    Ok(Collected::Step(Standing::Queued)) => Some(Ok(VideoStep::Queued)),
                    Ok(Collected::Step(Standing::Running { progress })) => Some(Ok(VideoStep::Running { progress })),
                    Ok(Collected::Step(Standing::Ended)) => None,
                    Ok(Collected::Made(Made::Video { video })) => Some(Ok(VideoStep::Made { video })),
                    Ok(Collected::Made(_)) => Some(Err(refused(MediaRefusal::Failed { message: "the job made no video".to_owned() }))),
                    Err(failure) => Some(Err(failure)),
                })
            })
            .boxed_local()
    }

    /// What the job of `of` made, once it ends, its steps passed over.
    pub(crate) async fn collected(&self, of: JobOfCall) -> Result<Made, PortFailure> {
        let mut steps = self.collect(of);
        while let Some(collected) = steps.next().await {
            match collected? {
                Collected::Step(_) => {}
                Collected::Made(made) => return Ok(made),
            }
        }
        Err(PortFailure::from(PortError::Ended("the job's steps ended before it did".into())))
    }

    /// Leaves the job of `call` to nobody once `cancel`, its call's stop,
    /// comes, unless the call comes back to it: the shard is closing, as a
    /// restart closes it, or the runner sends the call again
    /// ([`MediaCall::comes_back`]). A job that ended uncollected goes with
    /// its row; a call that collected its job finds none.
    pub(crate) fn watch_call_stop(&self, call: &MediaCall, cancel: CancellationToken) {
        let of = JobOfCall { call: call.id.clone(), ordinal: call.ordinal };
        let shard = self.this();
        let call = call.clone();
        self.tasks().spawn_local(async move {
            tokio::select! {
                biased;
                () = shard.closed() => return,
                () = cancel.cancelled() => {}
            }
            if call.comes_back() {
                tracing::info!(event = "media_job.kept_for_call", "a stopped call's media job goes on for its call's return");
                return;
            }
            let followed = shard.media_jobs.0.borrow().get(&of).cloned();
            match followed {
                Some(followed) => followed.abandon(),
                None => {
                    if let Err(error) = shard.services().control.delete_media_job(of).await {
                        tracing::warn!(event = "media_job.delete_failed", %error, "a stopped call's media job was not deleted");
                    }
                }
            }
        });
    }
}

/// What a job's collector yields: a change of its state, then what it made.
pub(crate) enum Collected {
    Step(Standing),
    Made(Made),
}

/// A finished job's file as one read of it answers, by its kind.
enum Finished {
    Video(Done),
    Image(demi_provider_common::images::MadeImage),
    Audio(demi_provider_common::audio::MadeAudio),
}

/// One read of `job`, by its maker's role of the job's kind.
async fn read(job: &MediaJob, reader: &Reader, catalog: &Catalog) -> MediaAnswer<JobState<Finished>> {
    let id = JobRef { id: &job.job, model: &job.usage.model, catalog };
    let none = || MediaAnswer { result: Err(unreadable(format!("its provider makes no {} as jobs", job.kind.plural()))), observed: Vec::new() };
    match &job.follow.answer {
        Made::Video { .. } => match reader.maker.videos() {
            Some(videos) => videos.state(&reader.http, id).await.and_then(|state| Ok(state.map(Finished::Video))),
            None => none(),
        },
        Made::Image { .. } => match reader.maker.images().and_then(|images| images.jobs()) {
            Some(jobs) => jobs.state(&reader.http, id).await.and_then(|state| Ok(state.map(Finished::Image))),
            None => none(),
        },
        Made::Audio { of, .. } => match reader.maker.audio().and_then(|audio| audio.jobs(catalog, *of, &job.usage.model)) {
            Some(jobs) => jobs.state(&reader.http, id).await.and_then(|state| Ok(state.map(Finished::Audio))),
            None => none(),
        },
    }
}

/// `made` with its file's bytes.
fn with_bytes(made: Made, bytes: B64Bytes) -> Made {
    match made {
        Made::Image { image } => Made::Image { image: MadeImage { bytes, ..image } },
        Made::Video { video } => Made::Video { video: MadeVideo { bytes, ..video } },
        Made::Audio { audio, of } => Made::Audio { audio: MadeAudio { bytes, ..audio }, of },
    }
}

/// Starts the shard of each user with a running job, whose start follows
/// them.
pub async fn start_media_followers(
    control: &demi_backend_database::control::ControlService,
    shards: &super::Shards,
) -> Result<(), demi_backend_database::StorageError> {
    for user in control.media_job_owners().await? {
        // A shard that cannot be reached follows them at its next call.
        if let Err(error) = shards.of(&user).call(|_, _| async {}).await {
            tracing::warn!(event = "media_job.start_failed", %user, %error, "the media jobs were not followed");
        }
    }
    Ok(())
}

/// Keeps the windows of the account's quota a response reported.
fn observe(row: &mut UsageRow, observed: &[QuotaWindow]) {
    if !observed.is_empty() {
        merge_windows(row.quota.get_or_insert_with(Vec::new), observed);
    }
}

