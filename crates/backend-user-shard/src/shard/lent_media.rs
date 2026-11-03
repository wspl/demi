//! The media of an entry lent to another server
//! (`crates/provider-relay/src/wire/media.rs`).
//!
//! A borrower's call runs on the shard of the key's maker as a call of theirs
//! does, on the entry's key or its accounts in turn (`super::media`), with no
//! usage ledger row: the usage is the borrower's. What crosses back is what
//! the vendor made, the counts and cost it reported, and a failure's kind and
//! code.
//!
//! A video's job is kept here under an id of its own, so the vendor's id and
//! the account that asked stay with the lender. The borrower reads its state
//! and downloads its video by that id; [`LentJobs`] forgets a job once its
//! video is downloaded, it is cancelled or its key revoked, or it is older
//! than any borrower still reads it.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use demi_plugin_interface::MediaRefusal;
use demi_provider_common::ProviderHttp;
use demi_provider_common::audio::{AudioRequest, AudioFormatAsk};
use demi_provider_common::images::{Aspect, InputImage};
use demi_provider_common::media::{MediaFailure, MediaModelInfo};
use demi_provider_common::media_catalog::Catalog;
use demi_provider_common::videos::{Done, JobRef, JobState, VideoRequest};
use demi_provider_registry::registry::MakerOf;
use demi_provider_relay::wire::media::{
    LentMaker, LentMediaModel, Takes, WireAskedVideo, WireAudioRequest, WireImageRequest, WireJobState, WireMadeAudio,
    WireMadeImage, WireMediaFailure, WirePcm, WireVideoRequest, WireVoice, WireVoicesRequest,
};
use demi_shared_types::{AudioKind, MediaKind, VideoResolution};
use tokio::time::Instant;

use super::Shard;
use super::media::{Route, Unmade, audio_kinds, model_of, on_accounts, route};
use crate::provider_sharing::Borrowing;

/// The video jobs borrowers asked for, by the id the borrower knows each by.
#[derive(Default)]
pub struct LentJobs(Mutex<HashMap<String, LentJob>>);

#[derive(Clone)]
struct LentJob {
    /// The id of the share key whose borrower asked for it.
    key: String,
    maker: MakerOf,
    http: ProviderHttp,
    catalog: Arc<Catalog>,
    /// The job's id at the vendor.
    vendor_job: String,
    model: String,
    asked_at: Instant,
    /// Where the finished job's video is.
    done: Option<Done>,
}

impl LentJob {
    /// Cancels the job at the vendor, for a vendor that cancels one; nothing
    /// awaits it.
    fn cancel(self) {
        tokio::spawn(async move {
            let job = JobRef { id: &self.vendor_job, model: &self.model, catalog: &self.catalog };
            let Some(cancel) = self.maker.videos().and_then(|videos| videos.cancel(&self.http, job)) else {
                return;
            };
            if let Err(failure) = cancel.await.result {
                tracing::info!(event = "relay.media_cancel_failed", key = self.key, code = failure.outcome(), "a lent video's job was not cancelled at the vendor");
            }
        });
    }
}

impl LentJobs {
    fn jobs(&self) -> std::sync::MutexGuard<'_, HashMap<String, LentJob>> {
        self.0.lock().expect("the lent jobs' lock is never poisoned")
    }

    /// Keeps `job` and answers its id, forgetting the jobs asked for before
    /// `stale`.
    fn keep(&self, job: LentJob, stale: Instant) -> String {
        let id = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 12]>());
        let mut jobs = self.jobs();
        jobs.retain(|_, kept| kept.asked_at >= stale);
        jobs.insert(id.clone(), job);
        id
    }

    /// Job `id`, when it is `key`'s.
    fn of(&self, key: &str, id: &str) -> Option<LentJob> {
        self.jobs().get(id).filter(|job| job.key == key).cloned()
    }

    fn take(&self, key: &str, id: &str) -> Option<LentJob> {
        let mut jobs = self.jobs();
        jobs.get(id).filter(|job| job.key == key)?;
        jobs.remove(id)
    }

    fn finished(&self, id: &str, done: &Done) {
        if let Some(job) = self.jobs().get_mut(id) {
            job.done = Some(done.clone());
        }
    }

    /// Cancels and forgets the jobs of the key `key`, which was revoked.
    pub(crate) fn revoke(&self, key: &str) {
        let mut jobs = self.jobs();
        let ids: Vec<String> = jobs.iter().filter(|(_, job)| job.key == key).map(|(id, _)| id.clone()).collect();
        for id in ids {
            if let Some(job) = jobs.remove(&id) {
                job.cancel();
            }
        }
    }
}

/// The failure a borrower is told of `unmade`.
fn told(unmade: Unmade) -> WireMediaFailure {
    match unmade {
        Unmade::Vendor { failure, .. } => WireMediaFailure::from(&failure),
        Unmade::Refusal(MediaRefusal::UsedUp { .. }) => WireMediaFailure::UsageLimit,
        Unmade::Refusal(_) => WireMediaFailure::Unanswered { code: None },
    }
}

fn vendor(failure: MediaFailure) -> Unmade {
    Unmade::Vendor { failure, label: None }
}

/// A video request that gives every option, whose unsent ones are those
/// a model's request lacks.
fn every_video_option(model: &str) -> VideoRequest {
    let image = || InputImage { media_type: "image/png", bytes: Bytes::new() };
    VideoRequest {
        model: model.to_owned(),
        prompt: String::new(),
        image: Some(image()),
        last_frame: Some(image()),
        references: vec![image()],
        duration: Some(1),
        aspect: Some(Aspect { width: 1, height: 1 }),
        resolution: Some(VideoResolution::Height(720)),
        audio: Some(true),
        seed: Some(0),
    }
}

/// An audio request of `kind` that gives every option.
fn every_audio_option(model: &str, kind: AudioKind) -> AudioRequest {
    AudioRequest {
        kind,
        model: model.to_owned(),
        text: String::new(),
        voice: None,
        instructions: Some(String::new()),
        speed: Some(1.0),
        duration: Some(1.0),
        instrumental: true,
        seed: Some(0),
        looped: true,
        format: AudioFormatAsk::default(),
    }
}

impl Shard {
    /// The lent entry's route for `kind`, and whether it makes them now.
    async fn lent_route(&self, borrowing: &Borrowing, kind: MediaKind) -> Result<(Route, bool), WireMediaFailure> {
        let services = self.services();
        let entry = borrowing.entry.clone();
        let makes = services.provider_registry.media_shapes(&entry).makes(kind) && entry.generates(kind);
        let preferences = services.control.preferences(self.user().clone()).await.map_err(|_| WireMediaFailure::Unanswered { code: None })?;
        match route(services, entry, &preferences, kind).await {
            Ok(Some(route)) => Ok((route, makes)),
            Ok(None) => Err(WireMediaFailure::Unmade),
            Err(_) => Err(WireMediaFailure::Unanswered { code: None }),
        }
    }

    /// The lent entry's route for a call of `kind`, while it makes them.
    async fn lent_call(&self, borrowing: &Borrowing, kind: MediaKind) -> Result<Route, WireMediaFailure> {
        match self.lent_route(borrowing, kind).await? {
            (route, true) => Ok(route),
            (_, false) => Err(WireMediaFailure::Unmade),
        }
    }

    /// What the lent entry's maker of `kind` says of itself and its models;
    /// no models while its switch is off, or when it makes none of `kind`,
    /// so a borrower lists none rather than failing to list.
    pub async fn lent_maker(self: Rc<Self>, borrowing: Borrowing, kind: MediaKind) -> Result<LentMaker, WireMediaFailure> {
        let (route, makes) = match self.lent_route(&borrowing, kind).await {
            Err(WireMediaFailure::Unmade) => {
                return Ok(LentMaker {
                    vendor_name: borrowing.entry.label.clone(),
                    vendor: None,
                    confirms_model: true,
                    needs_voice: false,
                    speeds: None,
                    models: Vec::new(),
                });
            }
            route => route?,
        };
        let services = self.services();
        let catalog = services.media.current();
        let media = route.maker.media();
        let listed = match makes {
            true => media
                .models(&route.http(services, self.http()), &catalog)
                .await
                .map_err(|failure| WireMediaFailure::from(&failure))?,
            false => Vec::new(),
        };
        let audio = route.maker.audio();
        let models = listed
            .into_iter()
            .map(|model| {
                let takes = match kind {
                    MediaKind::Image => Vec::new(),
                    MediaKind::Video => {
                        let videos = route.maker.videos().expect("a maker of videos has their role");
                        let lacks = videos.unsent(&catalog, &every_video_option(&model.id));
                        vec![Takes {
                            kind: None,
                            shape_name: videos.shape_name(),
                            lacks: lacks.into_iter().map(Into::into).collect(),
                        }]
                    }
                    MediaKind::Audio => {
                        let audio = audio.expect("a maker of audio has its role");
                        let kinds = audio_kinds(&catalog, media.vendor(), &model);
                        let kinds = if kinds.is_empty() { AudioKind::ALL.to_vec() } else { kinds };
                        kinds
                            .into_iter()
                            .map(|kind| {
                                let request = every_audio_option(&model.id, kind);
                                Takes {
                                    kind: Some(kind),
                                    shape_name: audio.shape_name(&catalog, &request),
                                    lacks: audio.unsent(&catalog, &request).into_iter().map(Into::into).collect(),
                                }
                            })
                            .collect()
                    }
                };
                let kinds = match kind {
                    MediaKind::Audio => audio_kinds(&catalog, media.vendor(), &model),
                    MediaKind::Image | MediaKind::Video => Vec::new(),
                };
                let MediaModelInfo { id, name, released, description, kinds: _ } = model;
                LentMediaModel { id, name, released, description, kinds, takes }
            })
            .collect();
        Ok(LentMaker {
            vendor_name: media.name().to_owned(),
            vendor: media.vendor().map(|vendor| vendor.id().to_owned()),
            confirms_model: media.confirms_model(),
            needs_voice: audio.is_some_and(|audio| audio.needs_voice()),
            speeds: audio.and_then(|audio| audio.speeds()).map(|(slowest, fastest)| [slowest, fastest]),
            models,
        })
    }

    /// Makes one image the borrower asked for.
    pub async fn lent_image(self: Rc<Self>, borrowing: Borrowing, request: WireImageRequest) -> Result<WireMadeImage, WireMediaFailure> {
        let request = request.request().ok_or(WireMediaFailure::Unmade)?;
        let route = self.lent_call(&borrowing, MediaKind::Image).await?;
        let services = self.services();
        let catalog = services.media.current();
        let http = route.http(services, self.http());
        let made = on_accounts(&self, &route, &model_of(&request.model), false, async |maker, _, _| {
            let images = maker.images().expect("a maker of images has their role");
            images.make(&http, &catalog, request.clone()).await.result.map_err(vendor)
        })
        .await;
        let made = self.lent_ended(&borrowing, "image", made)?;
        Ok(WireMadeImage { data: made.bytes.into(), usage: made.usage })
    }

    /// Makes one file of audio the borrower asked for.
    pub async fn lent_audio(self: Rc<Self>, borrowing: Borrowing, request: WireAudioRequest) -> Result<WireMadeAudio, WireMediaFailure> {
        let request = AudioRequest::from(request);
        let route = self.lent_call(&borrowing, MediaKind::Audio).await?;
        let services = self.services();
        let catalog = services.media.current();
        let http = route.http(services, self.http());
        let made = on_accounts(&self, &route, &model_of(&request.model), false, async |maker, _, _| {
            let audio = maker.audio().expect("a maker of audio has its role");
            audio.make(&http, &catalog, request.clone()).await.result.map_err(vendor)
        })
        .await;
        let made = self.lent_ended(&borrowing, "audio", made)?;
        Ok(WireMadeAudio {
            data: made.bytes.into(),
            pcm: made.pcm.map(|pcm| WirePcm { sample_rate: pcm.sample_rate(), channels: pcm.channels(), bits: pcm.bits() }),
            cost: made.cost,
            usage: made.usage,
        })
    }

    /// The voices a speech model of the lent entry speaks in.
    pub async fn lent_voices(
        self: Rc<Self>,
        borrowing: Borrowing,
        request: WireVoicesRequest,
    ) -> Result<Option<Vec<WireVoice>>, WireMediaFailure> {
        let route = self.lent_call(&borrowing, MediaKind::Audio).await?;
        let services = self.services();
        let catalog = services.media.current();
        let http = route.http(services, self.http());
        let voices = on_accounts(&self, &route, &model_of(&request.model), false, async |maker, _, _| {
            let audio = maker.audio().expect("a maker of audio has its role");
            audio.voices(&http, &catalog, &request.model, request.search.as_deref()).await.map_err(vendor)
        })
        .await;
        let voices = self.lent_ended(&borrowing, "voices", voices)?;
        Ok(voices.map(|voices| voices.into_iter().map(Into::into).collect()))
    }

    /// Asks for the video the borrower asked for, and keeps its job.
    pub async fn lent_video(self: Rc<Self>, borrowing: Borrowing, request: WireVideoRequest) -> Result<WireAskedVideo, WireMediaFailure> {
        let request = request.request().ok_or(WireMediaFailure::Unmade)?;
        let route = self.lent_call(&borrowing, MediaKind::Video).await?;
        let services = self.services();
        let catalog = services.media.current();
        let http = route.http(services, self.http());
        let asked = on_accounts(&self, &route, &model_of(&request.model), false, async |maker, _, _| {
            let videos = maker.videos().expect("a maker of videos has their role");
            let job = videos.ask(&http, &catalog, request.clone()).await.result.map_err(vendor)?;
            Ok((job, maker.clone()))
        })
        .await;
        let (vendor_job, maker) = self.lent_ended(&borrowing, "video", asked)?;
        let now = Instant::now();
        // A borrower stops reading a job once its own bound, the same as
        // this server's, has passed since its ask; twice that is past its
        // last read and its download.
        let stale = now.checked_sub(services.media_tuning.video_bound * 2).unwrap_or(now);
        let job = LentJob {
            key: borrowing.key.id.clone(),
            maker,
            http,
            catalog,
            vendor_job,
            model: request.model,
            asked_at: now,
            done: None,
        };
        Ok(WireAskedVideo { job: services.lent_jobs.keep(job, stale) })
    }

    /// The state of the borrower's video job `id`, read once.
    pub async fn lent_video_state(self: Rc<Self>, borrowing: Borrowing, id: String) -> Result<WireJobState, WireMediaFailure> {
        let services = self.services();
        let job = services.lent_jobs.of(&borrowing.key.id, &id).ok_or(WireMediaFailure::NoJob)?;
        let videos = job.maker.videos().expect("a kept job's maker makes videos");
        let read = JobRef { id: &job.vendor_job, model: &job.model, catalog: &job.catalog };
        let state = videos.state(&job.http, read).await.result;
        let state = state.map_err(|failure| WireMediaFailure::from(&failure))?;
        match &state {
            JobState::Done(done) => services.lent_jobs.finished(&id, done),
            JobState::Failed { .. } | JobState::Expired => drop(services.lent_jobs.take(&borrowing.key.id, &id)),
            JobState::Queued | JobState::Running { .. } => {}
        }
        Ok(WireJobState::from(&state))
    }

    /// The video of the borrower's finished job `id`, which is forgotten once
    /// it is downloaded.
    pub async fn lent_video_file(self: Rc<Self>, borrowing: Borrowing, id: String) -> Result<Bytes, WireMediaFailure> {
        let services = self.services();
        let job = services.lent_jobs.of(&borrowing.key.id, &id).ok_or(WireMediaFailure::NoJob)?;
        let done = job.done.as_ref().ok_or(WireMediaFailure::NoJob)?;
        let videos = job.maker.videos().expect("a kept job's maker makes videos");
        let video = videos.download(&job.http, done).await.result.map_err(|failure| WireMediaFailure::from(&failure))?;
        services.lent_jobs.take(&borrowing.key.id, &id);
        Ok(video)
    }

    /// Cancels the borrower's job `id` and forgets it.
    pub fn lent_video_cancel(&self, borrowing: &Borrowing, id: &str) -> Result<(), WireMediaFailure> {
        let job = self.services().lent_jobs.take(&borrowing.key.id, id).ok_or(WireMediaFailure::NoJob)?;
        job.cancel();
        Ok(())
    }

    /// `ended`'s result, logged by the key and the call's `purpose`; a
    /// failure as the borrower is told it.
    fn lent_ended<T>(&self, borrowing: &Borrowing, purpose: &'static str, ended: Result<T, Unmade>) -> Result<T, WireMediaFailure> {
        let key = &borrowing.key.id;
        match ended {
            Ok(made) => {
                tracing::info!(event = "relay.media_ended", key, purpose, outcome = "answered", "a lent media call ended");
                Ok(made)
            }
            Err(unmade) => {
                let outcome = match &unmade {
                    Unmade::Vendor { failure, .. } => failure.outcome(),
                    Unmade::Refusal(refusal) => format!("{refusal:?}"),
                };
                tracing::info!(event = "relay.media_ended", key, purpose, outcome, "a lent media call ended");
                Err(told(unmade))
            }
        }
    }
}
