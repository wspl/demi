//! The Hailuo shape: MiniMax's videos through its key, on the entry's origin,
//! each model by the request the catalog's `api` names for it.
//!
//! # `v2`, the H3 models
//!
//! - `POST /v2/video_generation` with `model` and `content` (the prompt its
//!   `text` item and each image an `image_url` item with its role) answers the
//!   task's `task_id`
//!   ([Create](https://platform.minimax.io/docs/api-reference/video-generation-v2-create)).
//! - `GET /v2/query/video_generation/<task_id>` answers its `task.status`,
//!   `queued`, `running`, `succeeded`, `failed` or `cancelled`, and once
//!   succeeded its `task.content.url`
//!   ([Query](https://platform.minimax.io/docs/api-reference/video-generation-v2-query)).
//! - v2 needs a resolution, a duration and, for a video from text alone, a
//!   ratio. A call that asks none sends `768P`, the model's shortest duration
//!   and `16:9`.
//! - Its refusals are HTTP statuses with an `error`.
//! - `DELETE /v2/video_generation/<task_id>` cancels a queued task and deletes
//!   the record of a finished one, so a stopped call reads the task first and
//!   deletes only a queued one ([Cancel or
//!   delete](https://platform.minimax.io/docs/api-reference/video-generation-v2-delete)).
//!
//! # `v1`, Hailuo 2.3 and 02
//!
//! - `POST /v1/video_generation` with `model`, `prompt` and the frames as
//!   `first_frame_image` and `last_frame_image` ([Text to
//!   video](https://platform.minimax.io/docs/api-reference/video-generation-t2v),
//!   [First and last
//!   frame](https://platform.minimax.io/docs/api-reference/video-generation-fl2v)).
//! - `GET /v1/query/video_generation?task_id=` answers `Preparing`, `Queueing`,
//!   `Processing`, `Success` or `Fail`
//!   ([Query](https://platform.minimax.io/docs/api-reference/video-generation-query)),
//!   and a finished task's `file_id`. `GET /v1/files/retrieve?file_id=` names
//!   its `file.download_url`, kept an hour ([Retrieve
//!   file](https://platform.minimax.io/docs/api-reference/file-management-retrieve)).
//! - v1 refuses with HTTP 200 and a `base_resp`, and MiniMax cancels no v1
//!   task.
//!
//! The video's address takes no header.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Secret, endpoint_url,
    images::InputImage,
    json_body,
    media::{Ask, Holding, MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, catalog_models, unreadable},
    media_catalog::{Catalog, MediaKind},
    videos::{self, Content, Done, JobRef, JobState, VideoMaker, VideoOption, VideoRequest, content, unsent_of},
};
use demi_shared_types::{Clock, VendorTrace, VideoResolution, VideoShape};
use futures_util::future::BoxFuture;
use reqwest::{Url, header::HeaderMap};
use serde::{Deserialize, Serialize};

use crate::minimax;

/// The resolution MiniMax documents as every model's default, sent when
/// none is asked, as v2 needs one.
const DEFAULT_RESOLUTION: &str = "768P";

/// The ratio a v2 video from text alone is sent with when none is asked,
/// as v2 needs one and takes no `adaptive` for it.
const DEFAULT_RATIO: &str = "16:9";

/// The request a MiniMax model takes, as the catalog's `api` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Api {
    V1,
    V2,
}

/// A MiniMax key's video role.
pub(crate) struct Hailuo {
    vendor: MediaVendor,
    key: Secret,
    origin: Url,
    clock: Arc<dyn Clock>,
}

impl Hailuo {
    pub(crate) fn new(vendor: MediaVendor, key: Secret, origin: Url, clock: Arc<dyn Clock>) -> Self {
        Self { vendor, key, origin, clock }
    }

    /// The request `model` takes, as `catalog` names it; a model it names
    /// none for, or one MiniMax's requests do not know, fails the call as a
    /// catalog that cannot be read.
    fn api(&self, catalog: &Catalog, model: &str) -> Result<Api, MediaFailure> {
        match catalog.model(self.vendor.id(), model).and_then(|model| model.api.as_deref()) {
            Some("v1") => Ok(Api::V1),
            Some("v2") => Ok(Api::V2),
            Some(other) => Err(unreadable(format!("the catalog of media models names a request \"{other}\" for {model} that MiniMax's requests do not know"))),
            None => Err(unreadable(format!("the catalog of media models names no request of MiniMax's for {model}"))),
        }
    }

    fn url(&self, path: &str) -> Url {
        endpoint_url(&self.origin, path)
    }

    /// Sends `ask` and answers the body of MiniMax's success.
    async fn send(&self, http: &demi_provider_common::ProviderHttp, ask: Ask) -> MediaAnswer<Bytes> {
        let headers = minimax::headers(&self.key);
        minimax::call(&*self.clock)
            .send(http, ask, Holding::Json, |_| std::future::ready(Ok((headers.clone(), None))))
            .await
            .and_then(|answered| Ok(answered.body))
    }

    /// Sends `ask` to a v1 endpoint, whose refusals are its `base_resp`.
    async fn send_v1(&self, http: &demi_provider_common::ProviderHttp, ask: Ask) -> MediaAnswer<Bytes> {
        minimax::send_checked(&*self.clock, http, &self.key, ask).await
    }

    fn v2_task(&self, task: &str) -> Url {
        self.url(&format!("/v2/video_generation/{task}"))
    }

    async fn read_state(&self, http: &demi_provider_common::ProviderHttp, api: Api, task: &str) -> MediaAnswer<JobState> {
        match api {
            Api::V2 => {
                let url = self.url(&format!("/v2/query/video_generation/{task}"));
                self.send(http, Ask::get(&url)).await.and_then(|body| read_v2_state(&body))
            }
            Api::V1 => {
                let mut url = self.url("/v1/query/video_generation");
                url.query_pairs_mut().append_pair("task_id", task);
                let read = self.send_v1(http, Ask::get(&url)).await;
                let mut observed = read.observed;
                let state = match read.result.and_then(|body| read_v1_state(&body)) {
                    Ok(V1State::Made { file_id }) => {
                        // A finished task names its file, whose address
                        // another read gives.
                        let mut url = self.url("/v1/files/retrieve");
                        url.query_pairs_mut().append_pair("file_id", &file_id);
                        let file = self.send_v1(http, Ask::get(&url)).await;
                        observed.extend(file.observed);
                        file.result.and_then(|body| read_file(&body))
                    }
                    Ok(V1State::Other(state)) => Ok(state),
                    Err(failure) => Err(failure),
                };
                MediaAnswer { result: state, observed }
            }
        }
    }
}

impl MediaMaker for Hailuo {
    fn name(&self) -> &str {
        self.vendor.name()
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(self.vendor)
    }

    /// The catalog's models of the key's MiniMax, which lists none.
    fn models<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(std::future::ready(Ok(catalog_models(catalog, self.vendor, MediaKind::Video))))
    }
}

impl VideoMaker for Hailuo {
    fn shape(&self) -> VideoShape {
        VideoShape::Hailuo
    }

    fn ask<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, catalog: &'a Catalog, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(async move {
            let api = match self.api(catalog, &request.model) {
                Ok(api) => api,
                Err(failure) => return MediaAnswer { result: Err(failure), observed: Vec::new() },
            };
            match api {
                Api::V2 => {
                    let body = Bytes::from(encode_v2(catalog, self.vendor, &request));
                    self.send(http, Ask::post_json(&self.url("/v2/video_generation"), body)).await.and_then(|body| read_asked(&body))
                }
                Api::V1 => {
                    let body = Bytes::from(encode_v1(&request));
                    self.send_v1(http, Ask::post_json(&self.url("/v1/video_generation"), body)).await.and_then(|body| read_asked(&body))
                }
            }
        })
    }

    fn state<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState>> {
        Box::pin(async move {
            match self.api(job.catalog, job.model) {
                Ok(api) => self.read_state(http, api, job.id).await,
                Err(failure) => MediaAnswer { result: Err(failure), observed: Vec::new() },
            }
        })
    }

    fn download<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, done: &'a Done) -> BoxFuture<'a, MediaAnswer<Bytes>> {
        Box::pin(videos::download(minimax::LABEL, &*self.clock, http, &done.url, &self.origin, HeaderMap::new()))
    }

    /// Deletes a v2 task while it is still queued; a running one runs to
    /// its end, and a finished one is left as it is. MiniMax cancels no v1
    /// task.
    fn cancel<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: JobRef<'a>) -> Option<BoxFuture<'a, MediaAnswer<()>>> {
        if self.api(job.catalog, job.model).ok()? != Api::V2 {
            return None;
        }
        Some(Box::pin(async move {
            let read = self.read_state(http, Api::V2, job.id).await;
            match read.result {
                Ok(JobState::Queued) => self.send(http, Ask::delete(&self.v2_task(job.id))).await.and_then(|_| Ok(())),
                Ok(_) => MediaAnswer { result: Ok(()), observed: read.observed },
                Err(failure) => MediaAnswer { result: Err(failure), observed: read.observed },
            }
        }))
    }

    /// v1 also takes no reference images and no shape; a model the catalog
    /// names no request for is told at its ask.
    fn unsent(&self, catalog: &Catalog, request: &VideoRequest) -> Vec<VideoOption> {
        match self.api(catalog, &request.model) {
            Ok(Api::V1) => unsent_of(&[VideoOption::Reference, VideoOption::Aspect, VideoOption::Audio, VideoOption::Seed], request),
            _ => videos::unsent(VideoShape::Hailuo, request),
        }
    }
}

/// MiniMax's resolution, `768P`, as it writes one.
fn resolution(asked: Option<VideoResolution>) -> Option<String> {
    asked.map(VideoResolution::uppercase)
}

#[derive(Serialize)]
struct V2Body<'a> {
    model: &'a str,
    content: Vec<Content>,
    resolution: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ratio: Option<String>,
}

/// A v2 ask: every option given in its field, and what v2 needs and the
/// call did not ask filled in, `768P`, the model's shortest duration as
/// `catalog` states it and, for a video from text alone, `16:9`.
fn encode_v2(catalog: &Catalog, vendor: MediaVendor, request: &VideoRequest) -> Vec<u8> {
    let from_text = request.image.is_none() && request.last_frame.is_none() && request.references.is_empty();
    let shortest = || {
        let video = catalog.model(vendor.id(), &request.model).and_then(|model| catalog.video(model))?;
        match (&video.durations, video.duration_range) {
            (Some(durations), _) => durations.iter().copied().min(),
            (None, Some(range)) => range.min.map(|min| min.ceil() as u32),
            (None, None) => None,
        }
    };
    json_body(&V2Body {
        model: &request.model,
        content: content(request),
        resolution: resolution(request.resolution).unwrap_or_else(|| DEFAULT_RESOLUTION.to_owned()),
        duration: request.duration.or_else(shortest),
        ratio: request.aspect.map(|aspect| aspect.to_string()).or_else(|| from_text.then(|| DEFAULT_RATIO.to_owned())),
    })
}

#[derive(Serialize)]
struct V1Body<'a> {
    model: &'a str,
    prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_frame_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_frame_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<String>,
}

/// A v1 ask, whose options left out are MiniMax's defaults.
fn encode_v1(request: &VideoRequest) -> Vec<u8> {
    json_body(&V1Body {
        model: &request.model,
        prompt: &request.prompt,
        first_frame_image: request.image.as_ref().map(InputImage::data_url),
        last_frame_image: request.last_frame.as_ref().map(InputImage::data_url),
        duration: request.duration,
        resolution: resolution(request.resolution),
    })
}

/// The task's id in the ask's answer.
fn read_asked(body: &[u8]) -> Result<String, MediaFailure> {
    #[derive(Deserialize)]
    struct Asked {
        task_id: String,
    }
    let asked: Asked =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    Ok(asked.task_id)
}

#[derive(Deserialize)]
struct V2Answer {
    task: V2Task,
}

#[derive(Deserialize)]
struct V2Task {
    status: String,
    #[serde(default)]
    content: Option<V2Content>,
    #[serde(default)]
    error: Option<V2Error>,
}

#[derive(Deserialize)]
struct V2Content {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct V2Error {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

fn read_v2_state(body: &[u8]) -> Result<JobState, MediaFailure> {
    let answer: V2Answer =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its task's state could not be read: {error}")))?;
    let task = answer.task;
    match task.status.as_str() {
        "queued" => Ok(JobState::Queued),
        "running" => Ok(JobState::Running { progress: None }),
        "failed" => {
            let error = task.error.unwrap_or(V2Error { code: None, message: None });
            Ok(JobState::Failed {
                message: error.message.filter(|message| !message.is_empty()).unwrap_or_else(|| "the task failed".to_owned()),
                trace: VendorTrace { code: error.code, id: None },
            })
        }
        "cancelled" => Ok(JobState::Failed { message: "the task was cancelled".to_owned(), trace: Default::default() }),
        "succeeded" => made(task.content.and_then(|content| content.url)),
        other => Err(unreadable(format!("its task's state \"{other}\" is not one it documents"))),
    }
}

/// A v1 task's state, or the file a finished one made.
enum V1State {
    Made { file_id: String },
    Other(JobState),
}

fn read_v1_state(body: &[u8]) -> Result<V1State, MediaFailure> {
    #[derive(Deserialize)]
    struct State {
        status: String,
        #[serde(default)]
        file_id: Option<String>,
    }
    let state: State =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its task's state could not be read: {error}")))?;
    match state.status.as_str() {
        "Preparing" | "Queueing" => Ok(V1State::Other(JobState::Queued)),
        "Processing" => Ok(V1State::Other(JobState::Running { progress: None })),
        "Fail" => Ok(V1State::Other(JobState::Failed { message: "the task failed".to_owned(), trace: Default::default() })),
        "Success" => match state.file_id.filter(|id| !id.is_empty()) {
            Some(file_id) => Ok(V1State::Made { file_id }),
            None => Err(unreadable("its finished task named no file".to_owned())),
        },
        other => Err(unreadable(format!("its task's state \"{other}\" is not one it documents"))),
    }
}

/// The video a v1 task made, as its file's retrieval names it.
fn read_file(body: &[u8]) -> Result<JobState, MediaFailure> {
    #[derive(Deserialize)]
    struct Retrieved {
        file: File,
    }
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        download_url: Option<String>,
    }
    let retrieved: Retrieved =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its file could not be read: {error}")))?;
    made(retrieved.file.download_url)
}

/// A finished task whose video is at `url`.
fn made(url: Option<String>) -> Result<JobState, MediaFailure> {
    let url = url.filter(|url| !url.is_empty()).ok_or_else(|| unreadable("its finished task named no video".to_owned()))?;
    let url = Url::parse(&url).map_err(|error| unreadable(format!("its video's address could not be read: {error}")))?;
    Ok(JobState::Done(Done { url, cost: None, usage: None }))
}
