//! A maker's calls as they cross a channel, such as the relay that lends an
//! entry of one Demi server to another (`crates/provider-relay`).
//!
//! The makers' traits are defined once, and so is how they cross: a [`Call`]
//! of each of their methods that sends a request, the one [`Reply`] it is
//! answered with, [`serve`], which answers a call with a maker's own
//! methods, and [`Remote`], which implements the traits by sending each
//! call through a [`Channel`]. A change of a trait changes these, here, and
//! no channel.
//!
//! ```text
//! caller's backend                         serving backend
//! Remote (ImageMaker, VideoMaker, ...)     serve(Roles, call)
//!   call ----> Channel ---- the wire ---->   the maker's own methods
//!   reply <---------------- the wire <----   its one reply
//! ```
//!
//! A video's job crosses as its methods do: its ask, then each read of its
//! state, and its cancel, each a call of its own, so the caller follows a
//! job across its own restart as it follows its own maker's.
//!
//! What a trait answers without a call, such as the options an audio model's
//! request lacks, crosses as [`Facts`], which [`Remote`] reads with each
//! listing of the models and answers those methods from.
//!
//! What stays with whoever serves is marked on the types themselves, which
//! skip it when they cross: a failure's words, trace and wait, and the
//! address of a finished video, which [`serve`] downloads and sends instead.
//! A job's id at the vendor is the server's to keep: whoever serves sends
//! one of its own in its place, which the caller's reads and cancel name.

use std::sync::{Arc, OnceLock, RwLock};

use bytes::Bytes;
use demi_shared_types::{AudioKind, MediaKind};
use futures_util::future::BoxFuture;
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::audio::{self, AudioFormatAsk, AudioMaker, AudioOption, AudioRequest, MadeAudio, Voice};
use crate::images::{ImageMaker, ImageRequest, MadeImage};
use crate::media::{MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, unreadable, within_limit};
use crate::media_catalog::Catalog;
use crate::videos::{Change, Done, DoneVideo, JobRef, JobState, VideoMaker, VideoRequest};

/// What a failed video whose words stayed with whoever served it says.
pub const NO_REASON: &str = "no reason is shared";

/// A call of a maker's, as it crosses.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub enum Call {
    /// What the maker says of itself and of each of its models.
    Facts,
    Image(ImageRequest),
    /// [`VideoMaker::ask`], answered with the job's id.
    VideoAsk(VideoRequest),
    /// [`VideoMaker::state`] of the job `job` asked of `model`.
    VideoState { job: String, model: String },
    /// [`VideoMaker::cancel`] of the job `job` asked of `model`.
    VideoCancel { job: String, model: String },
    Audio(AudioRequest),
    Voices { model: String, search: Option<String> },
}

impl Call {
    /// The reply that answers this call with `failure`.
    pub fn failed(&self, failure: MediaFailure) -> Reply {
        match self {
            Self::Facts => Reply::Facts(Err(failure)),
            Self::Image(_) => Reply::Image(Err(failure)),
            Self::VideoAsk(_) => Reply::VideoAsked(Err(failure)),
            Self::VideoState { .. } => Reply::VideoState(Err(failure)),
            Self::VideoCancel { .. } => Reply::VideoCancelled(Err(failure)),
            Self::Audio(_) => Reply::Audio(Err(failure)),
            Self::Voices { .. } => Reply::Voices(Err(failure)),
        }
    }

    /// The model the call names; none for [`Call::Facts`].
    pub fn model(&self) -> Option<&str> {
        match self {
            Self::Facts => None,
            Self::Image(request) => Some(&request.model),
            Self::VideoAsk(request) => Some(&request.model),
            Self::VideoState { model, .. } | Self::VideoCancel { model, .. } => Some(model),
            Self::Audio(request) => Some(&request.model),
            Self::Voices { model, .. } => Some(model),
        }
    }
}

/// The one answer to a [`Call`].
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub enum Reply {
    Facts(Result<Facts, MediaFailure>),
    Image(Result<MadeImage, MediaFailure>),
    VideoAsked(Result<String, MediaFailure>),
    /// A finished job's video is sent, never its address.
    VideoState(Result<JobState, MediaFailure>),
    VideoCancelled(Result<(), MediaFailure>),
    Audio(Result<MadeAudio, MediaFailure>),
    Voices(Result<Option<Vec<Voice>>, MediaFailure>),
}

impl Reply {
    /// Its failure, for a reply that is one.
    pub fn failure(&self) -> Option<&MediaFailure> {
        match self {
            Self::Facts(Err(failure))
            | Self::Image(Err(failure))
            | Self::VideoAsked(Err(failure))
            | Self::VideoState(Err(failure))
            | Self::VideoCancelled(Err(failure))
            | Self::Audio(Err(failure))
            | Self::Voices(Err(failure)) => Some(failure),
            _ => None,
        }
    }

    /// The reply with its failure, for one that is, changed by `change`.
    pub fn map_failure(self, change: impl FnOnce(MediaFailure) -> MediaFailure) -> Self {
        match self {
            Self::Facts(result) => Self::Facts(result.map_err(change)),
            Self::Image(result) => Self::Image(result.map_err(change)),
            Self::VideoAsked(result) => Self::VideoAsked(result.map_err(change)),
            Self::VideoState(result) => Self::VideoState(result.map_err(change)),
            Self::VideoCancelled(result) => Self::VideoCancelled(result.map_err(change)),
            Self::Audio(result) => Self::Audio(result.map_err(change)),
            Self::Voices(result) => Self::Voices(result.map_err(change)),
        }
    }
}

/// What a maker answers without a call.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Facts {
    /// [`MediaMaker::name`].
    pub name: String,
    /// The id of [`MediaMaker::vendor`].
    pub vendor: Option<String>,
    pub confirms_model: bool,
    pub needs_voice: bool,
    pub speeds: Option<(f64, f64)>,
    /// [`VideoMaker::changes`].
    pub changes: Vec<Change>,
    pub models: Vec<ModelFacts>,
}

/// A model, and what a request of it takes. A video model's options are
/// its catalog entry's, which the caller reads in its own catalog.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelFacts {
    pub info: MediaModelInfo,
    /// For an audio model, of each kind.
    pub audio: Vec<(AudioKind, Takes<AudioOption>)>,
}

/// What a request takes: how a call names it, and the options it has no
/// field for.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Takes<O> {
    pub shape_name: String,
    pub lacks: Vec<O>,
}

/// A maker of one kind, by its roles.
#[derive(Clone, Copy)]
pub struct Roles<'a> {
    pub media: &'a dyn MediaMaker,
    pub images: Option<&'a dyn ImageMaker>,
    pub videos: Option<&'a dyn VideoMaker>,
    pub audio: Option<&'a dyn AudioMaker>,
}

/// Answers `call` with `maker`'s own methods, a finished job's video sent
/// rather than its address. A maker that cancels no job answers its cancel
/// as done: the job runs to its end, as the maker's own would.
pub async fn serve(maker: Roles<'_>, http: &crate::ProviderHttp, catalog: &Catalog, call: Call) -> Reply {
    let none = |what: &str| unreadable(format!("it makes no {what}"));
    match call {
        Call::Facts => Reply::Facts(facts(maker, http, catalog).await),
        Call::Image(request) => match maker.images {
            Some(images) => Reply::Image(images.make(http, catalog, request).await.result),
            None => Reply::Image(Err(none("images"))),
        },
        Call::Audio(request) => match maker.audio {
            Some(audio) => Reply::Audio(audio.make(http, catalog, request).await.result),
            None => Reply::Audio(Err(none("audio"))),
        },
        Call::Voices { model, search } => match maker.audio {
            Some(audio) => Reply::Voices(audio.voices(http, catalog, &model, search.as_deref()).await),
            None => Reply::Voices(Err(none("audio"))),
        },
        Call::VideoAsk(request) => match maker.videos {
            Some(videos) => Reply::VideoAsked(videos.ask(http, catalog, request).await.result),
            None => Reply::VideoAsked(Err(none("videos"))),
        },
        Call::VideoState { job, model } => match maker.videos {
            Some(videos) => {
                let state = videos.state(http, JobRef { id: &job, model: &model, catalog }).await.result;
                Reply::VideoState(match state {
                    Ok(JobState::Done(done)) => sent(videos, http, done).await,
                    state => state,
                })
            }
            None => Reply::VideoState(Err(none("videos"))),
        },
        Call::VideoCancel { job, model } => match maker.videos {
            Some(videos) => match videos.cancel(http, JobRef { id: &job, model: &model, catalog }) {
                Some(cancel) => Reply::VideoCancelled(cancel.await.result),
                None => Reply::VideoCancelled(Ok(())),
            },
            None => Reply::VideoCancelled(Err(none("videos"))),
        },
    }
}

/// A finished job whose video is in it, downloaded when the vendor left it
/// at an address.
async fn sent(videos: &dyn VideoMaker, http: &crate::ProviderHttp, done: Done) -> Result<JobState, MediaFailure> {
    let video = match done.video {
        DoneVideo::Sent(bytes) => bytes,
        DoneVideo::At(url) => videos.download(http, &url).await.result?,
    };
    Ok(JobState::Done(Done { video: DoneVideo::Sent(video), ..done }))
}

/// What `maker` says of itself and of each of its models.
async fn facts(maker: Roles<'_>, http: &crate::ProviderHttp, catalog: &Catalog) -> Result<Facts, MediaFailure> {
    let listed = maker.media.models(http, catalog).await?;
    let models = listed
        .into_iter()
        .map(|info| {
            let audio = match maker.audio {
                Some(audio) => AudioKind::ALL
                    .into_iter()
                    .map(|kind| {
                        let request = every_audio_option(&info.id, kind);
                        let takes = Takes {
                            shape_name: audio.shape_name(catalog, &request),
                            lacks: audio.unsent(catalog, &request),
                        };
                        (kind, takes)
                    })
                    .collect(),
                None => Vec::new(),
            };
            ModelFacts { info, audio }
        })
        .collect();
    Ok(Facts {
        name: maker.media.name().to_owned(),
        vendor: maker.media.vendor().map(|vendor| vendor.id().to_owned()),
        confirms_model: maker.media.confirms_model(),
        needs_voice: maker.audio.is_some_and(|audio| audio.needs_voice()),
        speeds: maker.audio.and_then(|audio| audio.speeds()),
        changes: maker.videos.map(|videos| videos.changes()).unwrap_or_default(),
        models,
    })
}

/// What a maker of no kind it serves says of itself: no models, so a
/// caller lists none rather than failing to list.
pub fn no_facts(name: String) -> Facts {
    Facts { name, vendor: None, confirms_model: true, needs_voice: false, speeds: None, changes: Vec::new(), models: Vec::new() }
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

/// What carries a maker's calls to whoever serves them.
pub trait Channel: Send + Sync {
    /// The maker's name: the server's own, or once the server said it, of
    /// the vendor named `vendor`, such as `OpenAI on example.com`.
    fn name(&self, vendor: Option<&str>) -> String;

    /// Sends `call` to the maker of `kind`, with `http`, the entry's
    /// client, and answers its reply. A call the channel could not carry
    /// fails as the channel words it.
    fn call<'a>(&'a self, http: &'a crate::ProviderHttp, kind: MediaKind, call: Call) -> BoxFuture<'a, Result<Reply, MediaFailure>>;

    /// `failure` as the caller reads it: it crossed without its words.
    fn worded(&self, kind: MediaKind, failure: MediaFailure) -> MediaFailure;
}

/// A maker of `kind` whose calls a [`Channel`] carries.
pub struct Remote<C> {
    channel: Arc<C>,
    kind: MediaKind,
    /// Its name once the server said its vendor's.
    named: OnceLock<String>,
    /// Its name until then.
    unnamed: String,
    facts: RwLock<Option<Arc<Facts>>>,
}

impl<C: Channel> Remote<C> {
    pub fn new(channel: Arc<C>, kind: MediaKind) -> Self {
        let unnamed = channel.name(None);
        Self { channel, kind, named: OnceLock::new(), unnamed, facts: RwLock::new(None) }
    }

    fn facts(&self) -> Option<Arc<Facts>> {
        self.facts.read().expect("the facts' lock is never poisoned").clone()
    }

    fn model(&self, model: &str) -> Option<ModelFacts> {
        self.facts()?.models.iter().find(|listed| listed.info.id == model).cloned()
    }

    fn worded(&self, failure: MediaFailure) -> MediaFailure {
        self.channel.worded(self.kind, failure)
    }

    /// The one reply of `call`, which `pick` reads.
    async fn reply<T>(
        &self,
        http: &crate::ProviderHttp,
        call: Call,
        pick: impl FnOnce(Reply) -> Option<Result<T, MediaFailure>>,
    ) -> Result<T, MediaFailure> {
        let reply = self.channel.call(http, self.kind, call).await?;
        match pick(reply) {
            Some(result) => result.map_err(|failure| self.worded(failure)),
            None => Err(unreadable("it answered another call".to_owned())),
        }
    }

    fn answered<T>(result: Result<T, MediaFailure>) -> MediaAnswer<T> {
        MediaAnswer { result, observed: Vec::new() }
    }

    /// What a request of `request`'s model and kind takes.
    fn takes(&self, request: &AudioRequest) -> Option<Takes<AudioOption>> {
        let model = self.model(&request.model)?;
        model.audio.into_iter().find(|(kind, _)| *kind == request.kind).map(|(_, takes)| takes)
    }
}

impl<C: Channel> MediaMaker for Remote<C> {
    fn name(&self) -> &str {
        self.named.get().unwrap_or(&self.unnamed)
    }

    fn vendor(&self) -> Option<MediaVendor> {
        self.facts()?.vendor.as_deref().and_then(MediaVendor::from_id)
    }

    fn models<'a>(
        &'a self,
        http: &'a crate::ProviderHttp,
        _: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(async move {
            let facts = self
                .reply(http, Call::Facts, |reply| match reply {
                    Reply::Facts(facts) => Some(facts),
                    _ => None,
                })
                .await?;
            let _ = self.named.set(self.channel.name(Some(&facts.name)));
            let models = facts.models.iter().map(|model| model.info.clone()).collect();
            *self.facts.write().expect("the facts' lock is never poisoned") = Some(Arc::new(facts));
            Ok(models)
        })
    }

    fn confirms_model(&self) -> bool {
        self.facts().is_none_or(|facts| facts.confirms_model)
    }
}

impl<C: Channel> ImageMaker for Remote<C> {
    fn make<'a>(&'a self, http: &'a crate::ProviderHttp, _: &'a Catalog, request: ImageRequest) -> BoxFuture<'a, MediaAnswer<MadeImage>> {
        Box::pin(async move {
            let made = self
                .reply(http, Call::Image(request), |reply| match reply {
                    Reply::Image(made) => Some(made),
                    _ => None,
                })
                .await;
            Self::answered(made.and_then(|made| Ok(MadeImage { bytes: within_limit(made.bytes, "image")?, ..made })))
        })
    }
}

impl<C: Channel> VideoMaker for Remote<C> {
    /// Answers the id whoever serves gave the job in place of the vendor's.
    fn ask<'a>(&'a self, http: &'a crate::ProviderHttp, _: &'a Catalog, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(async move {
            let asked = self
                .reply(http, Call::VideoAsk(request), |reply| match reply {
                    Reply::VideoAsked(job) => Some(job),
                    _ => None,
                })
                .await;
            Self::answered(asked)
        })
    }

    fn state<'a>(&'a self, http: &'a crate::ProviderHttp, job: JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState>> {
        Box::pin(async move {
            let call = Call::VideoState { job: job.id.to_owned(), model: job.model.to_owned() };
            let state = self
                .reply(http, call, |reply| match reply {
                    Reply::VideoState(state) => Some(state),
                    _ => None,
                })
                .await;
            Self::answered(state.map(|state| match state {
                JobState::Failed { trace, .. } => JobState::Failed { reason: Some(NO_REASON.to_owned()), trace },
                state => state,
            }))
        })
    }

    fn cancel<'a>(&'a self, http: &'a crate::ProviderHttp, job: JobRef<'a>) -> Option<BoxFuture<'a, MediaAnswer<()>>> {
        Some(Box::pin(async move {
            let call = Call::VideoCancel { job: job.id.to_owned(), model: job.model.to_owned() };
            let cancelled = self
                .reply(http, call, |reply| match reply {
                    Reply::VideoCancelled(cancelled) => Some(cancelled),
                    _ => None,
                })
                .await;
            Self::answered(cancelled)
        }))
    }

    /// Those of the maker that serves, whose source goes as its file: a
    /// job it continues is named by an id the lender keeps to itself.
    fn changes(&self) -> Vec<Change> {
        self.facts().map(|facts| facts.changes.clone()).unwrap_or_default()
    }

    /// Never asked: whoever serves sends a finished job's video.
    fn download<'a>(&'a self, _: &'a crate::ProviderHttp, _: &'a Url) -> BoxFuture<'a, MediaAnswer<Bytes>> {
        Box::pin(async { Self::answered(Err(unreadable("it sent an address rather than its video".to_owned()))) })
    }
}

impl<C: Channel> AudioMaker for Remote<C> {
    fn needs_voice(&self) -> bool {
        self.facts().is_some_and(|facts| facts.needs_voice)
    }

    fn make<'a>(&'a self, http: &'a crate::ProviderHttp, _: &'a Catalog, request: AudioRequest) -> BoxFuture<'a, MediaAnswer<MadeAudio>> {
        Box::pin(async move {
            let made = self
                .reply(http, Call::Audio(request), |reply| match reply {
                    Reply::Audio(made) => Some(made),
                    _ => None,
                })
                .await;
            Self::answered(made.and_then(|made| Ok(MadeAudio { bytes: within_limit(made.bytes, "audio")?, ..made })))
        })
    }

    fn voices<'a>(
        &'a self,
        http: &'a crate::ProviderHttp,
        _: &'a Catalog,
        model: &'a str,
        search: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<Vec<Voice>>, MediaFailure>> {
        let call = Call::Voices { model: model.to_owned(), search: search.map(str::to_owned) };
        Box::pin(self.reply(http, call, |reply| match reply {
            Reply::Voices(voices) => Some(voices),
            _ => None,
        }))
    }

    fn unsent(&self, _: &Catalog, request: &AudioRequest) -> Vec<AudioOption> {
        let lacks = self.takes(request).map(|takes| takes.lacks).unwrap_or_default();
        audio::unsent_of(&lacks, request)
    }

    fn shape_name(&self, _: &Catalog, request: &AudioRequest) -> String {
        self.takes(request).map_or_else(|| self.name().to_owned(), |takes| takes.shape_name)
    }

    fn speeds(&self) -> Option<(f64, f64)> {
        self.facts()?.speeds
    }
}
