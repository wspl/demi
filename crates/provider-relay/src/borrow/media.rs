//! A borrowed entry's makers of media. Each call goes to the lender's routes
//! (`crate::wire::media`); what a maker is asked between calls, such as the
//! options a model's request lacks, is read from what the lender last said
//! of its maker, which each listing of its models reads again.

use std::sync::{Arc, OnceLock, RwLock};

use bytes::Bytes;
use demi_provider_common::audio::{self, AudioMaker, AudioOption, AudioRequest, MadeAudio, Voice};
use demi_provider_common::images::{ImageMaker, ImageRequest, MadeImage};
use demi_provider_common::media::{
    Holding, MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, read_within, transport, unreadable,
    within_limit,
};
use demi_provider_common::media_catalog::Catalog;
use demi_provider_common::videos::{self, Done, JobRef, JobState, VideoMaker, VideoOption, VideoRequest};
use demi_provider_common::{ErrorCode, ProviderHttp, RequestBuilder};
use demi_shared_types::audio::Pcm;
use demi_shared_types::{AudioKind, MediaKind, VendorTrace};
use futures_util::future::BoxFuture;
use http::Method;
use serde::de::DeserializeOwned;

use super::{Config, Refusal};
use crate::wire::media::{
    LentMaker, MEDIA_ROUTE, Takes, WireAskedVideo, WireAudioRequest, WireImageRequest, WireJobState, WireMadeAudio,
    WireMadeImage, WireMediaFailure, WireVideoRequest, WireVoice, WireVoicesRequest,
};

/// What a failure the lender reports says of its cause: nothing.
const NO_REASON: &str = "no reason is shared";

/// The lent entry's maker of one kind.
pub(super) struct Relayed {
    config: Arc<Config>,
    kind: MediaKind,
    /// Its name, once the lender said its vendor's, such as `OpenAI on
    /// example.com`; the lender's host until then.
    name: OnceLock<String>,
    said: RwLock<Option<Arc<LentMaker>>>,
}

impl Relayed {
    pub(super) fn new(config: Arc<Config>, kind: MediaKind) -> Self {
        Self { config, kind, name: OnceLock::new(), said: RwLock::new(None) }
    }

    fn said(&self) -> Option<Arc<LentMaker>> {
        self.said.read().expect("the maker's lock is never poisoned").clone()
    }

    /// What a request of `model`, of `kind` for audio, takes.
    fn takes(&self, model: &str, kind: Option<AudioKind>) -> Option<Takes> {
        let said = self.said()?;
        let model = said.models.iter().find(|listed| listed.id == model)?;
        model.takes.iter().find(|takes| takes.kind == kind).cloned()
    }

    fn request(&self, http: &ProviderHttp, method: Method, path: &str) -> RequestBuilder {
        let url = self.config.route(&format!("{MEDIA_ROUTE}/{path}"));
        self.config.authorized(http.request(method, url))
    }

    fn post(&self, http: &ProviderHttp, path: &str, body: &impl serde::Serialize) -> RequestBuilder {
        let body = serde_json::to_vec(body).expect("a media request serializes");
        self.request(http, Method::POST, path)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(body)
    }

    /// Sends `request`: an answer that is no success fails as it says.
    async fn send(&self, request: RequestBuilder) -> Result<reqwest::Response, MediaFailure> {
        let response = request.send().await.map_err(|error| transport(error.without_url()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = read_within(response, Holding::Json, "answer").await?;
        if status == http::StatusCode::UNPROCESSABLE_ENTITY
            && let Ok(failure) = serde_json::from_slice::<WireMediaFailure>(&body)
        {
            return Err(self.failure(failure));
        }
        let refusal = Refusal::of(status, &body);
        Err(MediaFailure::Unanswered {
            cause: refusal.message(&self.config),
            code: refusal.code(),
            trace: VendorTrace::default(),
        })
    }

    /// The JSON answer of `request`; `what` names the file it holds.
    async fn answer<T: DeserializeOwned>(&self, request: RequestBuilder, what: &str) -> Result<T, MediaFailure> {
        let response = self.send(request).await?;
        let body = read_within(response, Holding::Json, what).await?;
        serde_json::from_slice(&body)
            .map_err(|_| unreadable(format!("{} answered what no lender answers", self.config.lender)))
    }

    /// The failure the lender reported, worded here.
    fn failure(&self, failure: WireMediaFailure) -> MediaFailure {
        let lender = &self.config.lender;
        let code = |code: Option<String>| {
            code.map(|code| {
                let Ok(code) = code.parse::<ErrorCode>();
                code
            })
        };
        let unanswered = |cause: String, code| MediaFailure::Unanswered { cause, code, trace: VendorTrace::default() };
        match failure {
            WireMediaFailure::UsageLimit => MediaFailure::UsageLimit { retry_at: None, trace: VendorTrace::default() },
            WireMediaFailure::Unusable => unanswered(format!("its account on {lender} could not be used"), None),
            WireMediaFailure::Refused { code: said } => {
                MediaFailure::Refused { message: NO_REASON.to_owned(), code: code(said), trace: VendorTrace::default() }
            }
            WireMediaFailure::Shape { takes, code: said } => {
                MediaFailure::Shape { takes, code: code(said), trace: VendorTrace::default() }
            }
            WireMediaFailure::Unanswered { code: said } => {
                let code = code(said);
                let cause = match code {
                    Some(ErrorCode::Overloaded | ErrorCode::RateLimit | ErrorCode::DeviceOffline) => {
                        "it is not available right now".to_owned()
                    }
                    _ => NO_REASON.to_owned(),
                };
                unanswered(cause, code)
            }
            WireMediaFailure::Unmade => unanswered(format!("{lender} makes no {} with it now", self.kind.plural()), None),
            WireMediaFailure::NoJob => unanswered(format!("{lender} no longer has the job"), None),
        }
    }

    /// `call`'s answer, with no quota windows: those are the lender's.
    async fn answered<T>(call: impl Future<Output = Result<T, MediaFailure>>) -> MediaAnswer<T> {
        MediaAnswer { result: call.await, observed: Vec::new() }
    }
}

impl MediaMaker for Relayed {
    fn name(&self) -> &str {
        self.name.get().map_or(&self.config.lender, String::as_str)
    }

    fn vendor(&self) -> Option<MediaVendor> {
        self.said()?.vendor.as_deref().and_then(MediaVendor::from_id)
    }

    fn models<'a>(&'a self, http: &'a ProviderHttp, _: &'a Catalog) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(async move {
            let kind = match self.kind {
                MediaKind::Image => "makers/image",
                MediaKind::Video => "makers/video",
                MediaKind::Audio => "makers/audio",
            };
            let said: LentMaker = self.answer(self.request(http, Method::GET, kind), "listing").await?;
            let _ = self.name.set(format!("{} on {}", said.vendor_name, self.config.lender));
            let models = said
                .models
                .iter()
                .map(|model| MediaModelInfo {
                    id: model.id.clone(),
                    name: model.name.clone(),
                    released: model.released.clone(),
                    description: model.description.clone(),
                    kinds: model.kinds.clone(),
                })
                .collect();
            *self.said.write().expect("the maker's lock is never poisoned") = Some(Arc::new(said));
            Ok(models)
        })
    }

    fn confirms_model(&self) -> bool {
        self.said().is_none_or(|said| said.confirms_model)
    }
}

impl ImageMaker for Relayed {
    fn make<'a>(&'a self, http: &'a ProviderHttp, _: &'a Catalog, request: ImageRequest) -> BoxFuture<'a, MediaAnswer<MadeImage>> {
        Box::pin(Self::answered(async move {
            let made: WireMadeImage = self.answer(self.post(http, "image", &WireImageRequest::from(&request)), "image").await?;
            Ok(MadeImage { bytes: within_limit(made.data.into_bytes(), "image")?, usage: made.usage, vendor_id: None })
        }))
    }
}

impl VideoMaker for Relayed {
    fn ask<'a>(&'a self, http: &'a ProviderHttp, _: &'a Catalog, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(Self::answered(async move {
            let asked: WireAskedVideo = self.answer(self.post(http, "video", &WireVideoRequest::from(&request)), "answer").await?;
            Ok(asked.job)
        }))
    }

    fn state<'a>(&'a self, http: &'a ProviderHttp, job: JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState>> {
        Box::pin(Self::answered(async move {
            let path = format!("video/{}", job.id);
            let state: WireJobState = self.answer(self.request(http, Method::GET, &path), "answer").await?;
            Ok(match state {
                WireJobState::Queued => JobState::Queued,
                WireJobState::Running { progress } => JobState::Running { progress },
                WireJobState::Done { cost, usage } => JobState::Done(Done {
                    url: self.config.route(&format!("{MEDIA_ROUTE}/{path}/file")),
                    cost,
                    usage,
                }),
                WireJobState::Failed => JobState::Failed { message: NO_REASON.to_owned(), trace: VendorTrace::default() },
                WireJobState::Expired => JobState::Expired,
            })
        }))
    }

    /// Downloads the video from the lender, whose route [`state`](Self::state)
    /// named.
    fn download<'a>(&'a self, http: &'a ProviderHttp, done: &'a Done) -> BoxFuture<'a, MediaAnswer<Bytes>> {
        Box::pin(Self::answered(async move {
            let response = self.send(self.config.authorized(http.get(done.url.clone()))).await?;
            read_within(response, Holding::File, "video").await
        }))
    }

    fn cancel<'a>(&'a self, http: &'a ProviderHttp, job: JobRef<'a>) -> Option<BoxFuture<'a, MediaAnswer<()>>> {
        Some(Box::pin(Self::answered(async move {
            self.send(self.request(http, Method::DELETE, &format!("video/{}", job.id))).await.map(drop)
        })))
    }

    fn unsent(&self, _: &Catalog, request: &VideoRequest) -> Vec<VideoOption> {
        let lacks = self.takes(&request.model, None).map(|takes| takes.lacks).unwrap_or_default();
        let lacks: Vec<VideoOption> = lacks.into_iter().filter_map(|option| option.video()).collect();
        videos::unsent_of(&lacks, request)
    }

    fn shape_name(&self) -> String {
        let named = self.said().and_then(|said| {
            said.models.iter().flat_map(|model| &model.takes).find(|takes| takes.kind.is_none()).map(|takes| takes.shape_name.clone())
        });
        named.unwrap_or_else(|| self.name().to_owned())
    }
}

impl AudioMaker for Relayed {
    fn needs_voice(&self) -> bool {
        self.said().is_some_and(|said| said.needs_voice)
    }

    fn make<'a>(&'a self, http: &'a ProviderHttp, _: &'a Catalog, request: AudioRequest) -> BoxFuture<'a, MediaAnswer<MadeAudio>> {
        Box::pin(Self::answered(async move {
            let made: WireMadeAudio = self.answer(self.post(http, "audio", &WireAudioRequest::from(&request)), "audio").await?;
            let pcm = match made.pcm {
                Some(pcm) => Some(
                    Pcm::new(pcm.sample_rate, pcm.channels, pcm.bits).map_err(|error| unreadable(format!("its {error}")))?,
                ),
                None => None,
            };
            Ok(MadeAudio {
                bytes: within_limit(made.data.into_bytes(), "audio")?,
                pcm,
                cost: made.cost,
                usage: made.usage,
                vendor_id: None,
            })
        }))
    }

    fn voices<'a>(
        &'a self,
        http: &'a ProviderHttp,
        _: &'a Catalog,
        model: &'a str,
        search: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<Vec<Voice>>, MediaFailure>> {
        Box::pin(async move {
            let request = WireVoicesRequest { model: model.to_owned(), search: search.map(str::to_owned) };
            let voices: Option<Vec<WireVoice>> = self.answer(self.post(http, "audio/voices", &request), "listing").await?;
            Ok(voices.map(|voices| voices.into_iter().map(Into::into).collect()))
        })
    }

    fn unsent(&self, _: &Catalog, request: &AudioRequest) -> Vec<AudioOption> {
        let lacks = self.takes(&request.model, Some(request.kind)).map(|takes| takes.lacks).unwrap_or_default();
        let lacks: Vec<AudioOption> = lacks.into_iter().filter_map(|option| option.audio()).collect();
        audio::unsent_of(&lacks, request)
    }

    fn shape_name(&self, _: &Catalog, request: &AudioRequest) -> String {
        match self.takes(&request.model, Some(request.kind)) {
            Some(takes) => takes.shape_name,
            None => self.name().to_owned(),
        }
    }

    fn speeds(&self) -> Option<(f64, f64)> {
        self.said()?.speeds.map(|[slowest, fastest]| (slowest, fastest))
    }
}
