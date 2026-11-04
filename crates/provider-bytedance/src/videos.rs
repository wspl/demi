//! The Ark Tasks shape, Seedance's through an Ark key.
//!
//! - `POST …/contents/generations/tasks` with `model` and `content` (the prompt
//!   its `text` item and each image an `image_url` item with its role) answers
//!   the task's `id` ([Create a video generation
//!   task](https://docs.byteplus.com/en/docs/modelark/create-video-generation-task-api)).
//! - `GET …/contents/generations/tasks/<id>` answers its `status`, `queued`,
//!   `running`, `succeeded`, `failed`, `cancelled` or `expired`. Once succeeded
//!   it answers its `content.video_url`, kept 24 hours and downloaded without a
//!   header, with the output tokens it was paid by and the `resolution` it
//!   was made in, which the catalog's rates price them at ([Retrieve a video
//!   generation
//!   task](https://docs.byteplus.com/en/docs/modelark/get-video-generation-task-api)).
//! - `DELETE` of a task cancels it while it is queued, the one state Ark
//!   cancels in, and deletes the record of a finished one. So a stopped call
//!   reads the task's state first and deletes only a queued one ([Cancel or
//!   delete a video generation
//!   task](https://docs.byteplus.com/en/docs/modelark/cancel-or-delete-video-generation-tasks-api)).
//!
//! A video comes as the vendor makes it for its API: `watermark: false`.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Secret, endpoint_url, request_id_in,
    json_body,
    media::{Ask, Holding, MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, catalog_models, unreadable},
    media_catalog::{Catalog, MediaKind, PriceUnit, Tokens},
    videos::{self, Content, Done, DoneVideo, JobRef, JobState, VideoMaker, VideoRequest, content},
};
use demi_shared_types::{Clock, TokenUsage, VendorTrace, VideoResolution};
use futures_util::future::BoxFuture;
use reqwest::{Url, header::HeaderMap};
use serde::{Deserialize, Serialize};

use crate::ark;

/// The video role of an Ark key, of BytePlus or of Volcengine, whose models
/// are the catalog's.
pub(crate) struct ArkVideos {
    vendor: MediaVendor,
    key: Secret,
    base: Url,
    clock: Arc<dyn Clock>,
}

impl ArkVideos {
    pub(crate) fn vendor(vendor: MediaVendor, key: Secret, base: Url, clock: Arc<dyn Clock>) -> Self {
        Self { vendor, key, base, clock }
    }

    fn tasks(&self, job: Option<&str>) -> Url {
        match job {
            Some(job) => endpoint_url(&self.base, &format!("/contents/generations/tasks/{job}")),
            None => endpoint_url(&self.base, "/contents/generations/tasks"),
        }
    }

    async fn send(&self, http: &demi_provider_common::ProviderHttp, ask: Ask) -> MediaAnswer<Bytes> {
        let headers = ark::headers(&self.key);
        ark::call(&*self.clock)
            .send(http, ask, Holding::Json, |_| std::future::ready(Ok((headers.clone(), None))))
            .await
            .and_then(|answered| Ok(answered.body))
    }

    async fn read_state(&self, http: &demi_provider_common::ProviderHttp, job: JobRef<'_>) -> MediaAnswer<JobState> {
        let price = job.catalog.model(self.vendor.id(), job.model).and_then(|known| job.catalog.price(known));
        self.send(http, Ask::get(&self.tasks(Some(job.id)))).await.and_then(|body| {
            read_state(&body).map(|state| match state {
                Read::Done { url, tokens, resolution } => {
                    let priced = [Tokens { unit: PriceUnit::MillionOutputTokens, modality: None, count: tokens.unwrap_or_default() }];
                    let cost = tokens.and(price).and_then(|price| price.of_tokens(resolution.as_deref(), &priced));
                    // Ark bills a video by the tokens it made; it reads no input.
                    let usage = tokens.map(|output_tokens| TokenUsage {
                        input_tokens: 0,
                        output_tokens,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                    });
                    JobState::Done(Done { video: DoneVideo::At(url), cost, usage })
                }
                Read::State(state) => state,
            })
        })
    }
}

impl MediaMaker for ArkVideos {
    fn name(&self) -> &str {
        self.vendor.name()
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(self.vendor)
    }

    /// The catalog's models of the key's Ark, which lists none to a key.
    fn models<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(std::future::ready(Ok(catalog_models(catalog, self.vendor, MediaKind::Video))))
    }
}

impl VideoMaker for ArkVideos {
    fn run<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a demi_provider_common::media_catalog::Catalog,
        request: VideoRequest,
        every: std::time::Duration,
    ) -> futures_util::stream::BoxStream<'a, MediaAnswer<demi_provider_common::videos::JobStep>> {
        demi_provider_common::videos::polled(self, http, catalog, request, every)
    }

    /// The video's address is temporary and takes no header.
    fn download<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, url: &'a reqwest::Url) -> BoxFuture<'a, MediaAnswer<Bytes>> {
        Box::pin(videos::download("Ark", &*self.clock, http, url, &self.base, HeaderMap::new()))
    }

    /// Deletes the task while it is still queued; a running one runs to its
    /// end, which Ark cannot stop, and a finished one is left as it is.
    fn cancel<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: JobRef<'a>) -> Option<BoxFuture<'a, MediaAnswer<()>>>  {
        Some(Box::pin(async move {
            let read = self.read_state(http, job).await;
            match read.result {
                Ok(JobState::Queued) => self.send(http, Ask::delete(&self.tasks(Some(job.id)))).await.and_then(|_| Ok(())),
                Ok(_) => MediaAnswer { result: Ok(()), observed: read.observed },
                Err(failure) => MediaAnswer { result: Err(failure), observed: read.observed },
            }
        }))
    }
}

impl demi_provider_common::videos::PolledVideos for ArkVideos {
    fn ask<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, _: &'a demi_provider_common::media_catalog::Catalog, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(async move {
            let body = Bytes::from(encode(&request));
            self.send(http, Ask::post_json(&self.tasks(None), body)).await.and_then(|body| read_asked(&body))
        })
    }

    fn state<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState>>  {
        Box::pin(self.read_state(http, job))
    }
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    content: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generate_audio: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    watermark: bool,
}

/// The body of the ask: every option given, each in its field.
fn encode(request: &VideoRequest) -> Vec<u8> {
    json_body(&Body {
        model: &request.model,
        content: content(request),
        duration: request.duration,
        ratio: request.aspect.map(|aspect| aspect.to_string()),
        resolution: request.resolution.map(VideoResolution::lowercase),
        generate_audio: request.audio,
        seed: request.seed,
        watermark: false,
    })
}

/// The task's id in the ask's answer.
fn read_asked(body: &[u8]) -> Result<String, MediaFailure> {
    #[derive(Deserialize)]
    struct Asked {
        id: String,
    }
    let asked: Asked =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    Ok(asked.id)
}

#[derive(Deserialize)]
struct State {
    status: String,
    #[serde(default)]
    content: Option<Video>,
    #[serde(default)]
    error: Option<TaskError>,
    #[serde(default)]
    usage: Option<Usage>,
    /// What a finished task was made in, such as `720p`.
    #[serde(default)]
    resolution: Option<String>,
}

/// What one read of a task says: a finished task's video, its tokens and
/// its resolution, which the maker prices, or any other state.
enum Read {
    Done { url: Url, tokens: Option<u64>, resolution: Option<String> },
    State(JobState),
}

#[derive(Deserialize)]
struct Video {
    #[serde(default)]
    video_url: Option<String>,
}

#[derive(Deserialize)]
struct TaskError {
    #[serde(default)]
    code: Option<String>,
    message: String,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    completion_tokens: Option<u64>,
}

/// The state one read of the task answered.
fn read_state(body: &[u8]) -> Result<Read, MediaFailure> {
    let state: State =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its task's state could not be read: {error}")))?;
    let other = |state| Ok(Read::State(state));
    match state.status.as_str() {
        "queued" => other(JobState::Queued),
        "running" => other(JobState::Running { progress: None }),
        "failed" => match state.error {
            // Ark's message quotes the request ID its support asks for.
            Some(TaskError { code, message }) => {
                let id = request_id_in(&message);
                other(JobState::Failed { reason: None, trace: VendorTrace { words: None, code, id }.with_words(Some(message)) })
            }
            None => other(JobState::Failed { reason: None, trace: VendorTrace::default() }),
        },
        "cancelled" => other(JobState::Failed { reason: Some("the task was cancelled".to_owned()), trace: Default::default() }),
        "expired" => other(JobState::Expired),
        "succeeded" => {
            let url = state
                .content
                .and_then(|content| content.video_url)
                .filter(|url| !url.is_empty())
                .ok_or_else(|| unreadable("its finished task named no video".to_owned()))?;
            let url = Url::parse(&url).map_err(|error| unreadable(format!("its video's address could not be read: {error}")))?;
            let tokens = state.usage.and_then(|usage| usage.completion_tokens);
            Ok(Read::Done { url, tokens, resolution: state.resolution })
        }
        other => Err(unreadable(format!("its task's state \"{other}\" is not one it documents"))),
    }
}
