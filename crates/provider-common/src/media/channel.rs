//! A maker's calls as they cross a channel, such as the relay that lends an
//! entry of one Demi server to another (`crates/provider-relay`).
//!
//! The makers' traits are defined once, and so is how they cross: a [`Call`]
//! of each of their methods that sends a request, the [`Reply`]s it is
//! answered with, [`serve`], which answers a call with a maker's own
//! methods, and [`Remote`], which implements the traits by sending each
//! call through a [`Channel`]. A change of a trait changes these, here, and
//! no channel.
//!
//! ```text
//! caller's backend                         serving backend
//! Remote (ImageMaker, VideoMaker, ...)     serve(Roles, call)
//!   call ----> Channel ---- the wire ---->   the maker's own methods
//!   replies <-------------- the wire <----   one reply, or a job's steps
//! ```
//!
//! What a trait answers without a call, such as the options a model's
//! request lacks, crosses as [`Facts`], which [`Remote`] reads with each
//! listing of the models and answers those methods from.
//!
//! What stays with whoever serves is marked on the types themselves, which
//! skip it when they cross: a failure's words, trace and wait, a vendor's
//! ids of what it made, and the address of a finished video, which [`serve`]
//! downloads and sends instead.

use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use bytes::Bytes;
use demi_shared_types::{AudioKind, MediaKind, VideoResolution};
use futures_util::future::BoxFuture;
use futures_util::stream::{self, BoxStream, StreamExt as _};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::audio::{self, AudioFormatAsk, AudioMaker, AudioOption, AudioRequest, MadeAudio, Voice};
use crate::images::{Aspect, ImageMaker, ImageRequest, InputImage, MadeImage};
use crate::media::{MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, unreadable, within_limit};
use crate::media_catalog::Catalog;
use crate::videos::{
    self, Change, Done, DoneVideo, JobState, JobStep, SourceVideo, VideoMaker, VideoOption, VideoRequest, VideoSource,
};

/// What a failed video whose words stayed with whoever served it says.
pub const NO_REASON: &str = "no reason is shared";

/// A call of a maker's, as it crosses.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub enum Call {
    /// What the maker says of itself and of each of its models.
    Facts,
    Image(ImageRequest),
    /// A video's whole job, answered with each of its steps.
    Video(VideoRequest),
    Audio(AudioRequest),
    Voices { model: String, search: Option<String> },
}

impl Call {
    /// The reply that answers this call with `failure`.
    pub fn failed(&self, failure: MediaFailure) -> Reply {
        match self {
            Self::Facts => Reply::Facts(Err(failure)),
            Self::Image(_) => Reply::Image(Err(failure)),
            Self::Video(_) => Reply::Video(Err(failure)),
            Self::Audio(_) => Reply::Audio(Err(failure)),
            Self::Voices { .. } => Reply::Voices(Err(failure)),
        }
    }

    /// The model the call names; none for [`Call::Facts`].
    pub fn model(&self) -> Option<&str> {
        match self {
            Self::Facts => None,
            Self::Image(request) => Some(&request.model),
            Self::Video(request) => Some(&request.model),
            Self::Audio(request) => Some(&request.model),
            Self::Voices { model, .. } => Some(model),
        }
    }
}

/// An answer to a [`Call`]: one for each, but for a video's job, which is
/// answered with each of its steps, the last of them its end.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub enum Reply {
    Facts(Result<Facts, MediaFailure>),
    Image(Result<MadeImage, MediaFailure>),
    Video(Result<JobStep, MediaFailure>),
    Audio(Result<MadeAudio, MediaFailure>),
    Voices(Result<Option<Vec<Voice>>, MediaFailure>),
}

impl Reply {
    /// Its failure, for a reply that is one.
    pub fn failure(&self) -> Option<&MediaFailure> {
        match self {
            Self::Facts(Err(failure))
            | Self::Image(Err(failure))
            | Self::Video(Err(failure))
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
            Self::Video(result) => Self::Video(result.map_err(change)),
            Self::Audio(result) => Self::Audio(result.map_err(change)),
            Self::Voices(result) => Self::Voices(result.map_err(change)),
        }
    }

    /// Whether it is a call's last: any but a video job's step that is
    /// not its end.
    pub fn is_last(&self) -> bool {
        !matches!(
            self,
            Self::Video(Ok(JobStep::Asked(_) | JobStep::State(JobState::Queued | JobState::Running { .. })))
        )
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

/// A model, and what a request of it takes.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModelFacts {
    pub info: MediaModelInfo,
    /// For a video model: of a new video, then of each of the maker's
    /// [`changes`](Facts::changes), whose request may take less.
    pub video: Vec<(Option<Change>, Takes<VideoOption>)>,
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

/// Answers `call` with `maker`'s own methods: one reply, or a video job's
/// steps, the video it made sent rather than its address. `every` paces
/// the reads of a vendor whose job is read.
pub fn serve<'a>(
    maker: Roles<'a>,
    http: &'a crate::ProviderHttp,
    catalog: &'a Catalog,
    call: Call,
    every: Duration,
) -> BoxStream<'a, Reply> {
    let none = |what: &str| unreadable(format!("it makes no {what}"));
    match call {
        Call::Facts => stream::once(async move { Reply::Facts(facts(maker, http, catalog).await) }).boxed(),
        Call::Image(request) => match maker.images {
            Some(images) => stream::once(async move { Reply::Image(images.make(http, catalog, request).await.result) }).boxed(),
            None => stream::iter([Reply::Image(Err(none("images")))]).boxed(),
        },
        Call::Audio(request) => match maker.audio {
            Some(audio) => stream::once(async move { Reply::Audio(audio.make(http, catalog, request).await.result) }).boxed(),
            None => stream::iter([Reply::Audio(Err(none("audio")))]).boxed(),
        },
        Call::Voices { model, search } => match maker.audio {
            Some(audio) => stream::once(async move {
                Reply::Voices(audio.voices(http, catalog, &model, search.as_deref()).await)
            })
            .boxed(),
            None => stream::iter([Reply::Voices(Err(none("audio")))]).boxed(),
        },
        Call::Video(request) => match maker.videos {
            Some(videos) => videos
                .run(http, catalog, request, every)
                .then(move |answer| async move {
                    Reply::Video(match answer.result {
                        Ok(JobStep::State(JobState::Done(done))) => sent(videos, http, done).await.map(JobStep::State),
                        step => step,
                    })
                })
                .boxed(),
            None => stream::iter([Reply::Video(Err(none("videos")))]).boxed(),
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
            let video = match maker.videos {
                Some(videos) => std::iter::once(None)
                    .chain(videos.changes().into_iter().map(Some))
                    .map(|change| {
                        let takes = Takes {
                            shape_name: videos.shape_name(),
                            lacks: videos.unsent(catalog, &every_video_option(&info.id, change)),
                        };
                        (change, takes)
                    })
                    .collect(),
                None => Vec::new(),
            };
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
            ModelFacts { info, video, audio }
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

/// A video request that gives every option, a new video's or `change`'s,
/// whose unsent ones are those a model's request lacks.
fn every_video_option(model: &str, change: Option<Change>) -> VideoRequest {
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
        source: change.map(|change| VideoSource { change, video: SourceVideo::File(Bytes::new()) }),
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

/// What carries a maker's calls to whoever serves them.
pub trait Channel: Send + Sync {
    /// The maker's name: the server's own, or once the server said it, of
    /// the vendor named `vendor`, such as `OpenAI on example.com`.
    fn name(&self, vendor: Option<&str>) -> String;

    /// Sends `call` to the maker of `kind`, with `http`, the entry's
    /// client, and answers its replies as they come. A call the channel
    /// could not carry fails as the channel words it.
    fn call<'a>(
        &'a self,
        http: &'a crate::ProviderHttp,
        kind: MediaKind,
        call: Call,
    ) -> BoxStream<'a, Result<Reply, MediaFailure>>;

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
        let mut replies = self.channel.call(http, self.kind, call);
        let reply = match replies.next().await {
            Some(reply) => reply?,
            None => return Err(unreadable("its answer ended before its reply".to_owned())),
        };
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
    fn run<'a>(
        &'a self,
        http: &'a crate::ProviderHttp,
        _: &'a Catalog,
        request: VideoRequest,
        _: Duration,
    ) -> BoxStream<'a, MediaAnswer<JobStep>> {
        self.channel
            .call(http, self.kind, Call::Video(request))
            .map(move |reply| {
                Self::answered(match reply {
                    Ok(Reply::Video(Ok(JobStep::State(JobState::Failed { trace, .. })))) => {
                        Ok(JobStep::State(JobState::Failed { reason: Some(NO_REASON.to_owned()), trace }))
                    }
                    Ok(Reply::Video(step)) => step.map_err(|failure| self.worded(failure)),
                    Ok(_) => Err(unreadable("it answered another call".to_owned())),
                    Err(failure) => Err(failure),
                })
            })
            .boxed()
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

    fn unsent(&self, _: &Catalog, request: &VideoRequest) -> Vec<VideoOption> {
        let change = request.source.as_ref().map(|source| source.change);
        let model = self.model(&request.model);
        let takes = model.and_then(|model| model.video.into_iter().find(|(of, _)| *of == change));
        let lacks = takes.map(|(_, takes)| takes.lacks).unwrap_or_default();
        videos::unsent_of(&lacks, request)
    }

    fn shape_name(&self) -> String {
        let named = self
            .facts()
            .and_then(|facts| facts.models.iter().find_map(|model| model.video.first().map(|(_, takes)| takes.shape_name.clone())));
        named.unwrap_or_else(|| self.name().to_owned())
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
