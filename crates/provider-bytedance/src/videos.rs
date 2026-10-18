//! The Ark Tasks shape (`videos.md` § Vendors), Seedance's through an Ark
//! key and any compatible API that chose it:
//! `POST …/contents/generations/tasks` with `model` and `content`, the
//! prompt its `text` item and each image an `image_url` item with its
//! role, answers the task's `id`
//! ([Create a video generation task](https://docs.byteplus.com/en/docs/modelark/create-video-generation-task-api));
//! `GET …/contents/generations/tasks/<id>` answers its `status`, `queued`,
//! `running`, `succeeded`, `failed`, `cancelled` or `expired`, and once
//! succeeded its `content.video_url`, kept 24 hours and downloaded without
//! a header, with the output tokens it was paid by
//! ([Retrieve a video generation task](https://docs.byteplus.com/en/docs/modelark/get-video-generation-task-api)).
//! `DELETE` of a task cancels it while it is queued, the one state Ark
//! cancels in, and deletes the record of a finished one, so a stopped call
//! reads the task's state first and deletes only a queued one
//! ([Cancel or delete a video generation task](https://docs.byteplus.com/en/docs/modelark/cancel-or-delete-video-generation-tasks-api)).
//! A video comes as the vendor makes it for its API: `watermark: false`.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Secret, endpoint_url, request_id_in,
    json_body,
    media::{Ask, Holding, MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, catalog_models, unreadable},
    media_catalog::{Catalog, MediaKind},
    videos::{self, Content, Done, JobState, VideoMaker, VideoRequest, content},
};
use demi_shared_types::{Clock, TokenUsage, VendorTrace, VideoResolution, VideoShape};
use futures_util::future::BoxFuture;
use reqwest::{Url, header::HeaderMap};
use serde::{Deserialize, Serialize};

use crate::ark;

/// Whose Ark an entry's tasks go to.
enum Flavor {
    /// An Ark key's, whose models are the catalog's.
    Vendor(MediaVendor),
    /// A compatible endpoint, which the user knows by the entry's name,
    /// whose models are the ones the user added.
    Compatible { name: String, models: Vec<MediaModelInfo> },
}

/// The video role of an Ark key or of a compatible endpoint that speaks
/// Ark Tasks.
pub(crate) struct ArkVideos {
    flavor: Flavor,
    /// None for a compatible endpoint that takes none.
    key: Option<Secret>,
    base: Url,
    clock: Arc<dyn Clock>,
}

impl ArkVideos {
    pub(crate) fn vendor(vendor: MediaVendor, key: Secret, base: Url, clock: Arc<dyn Clock>) -> Self {
        Self { flavor: Flavor::Vendor(vendor), key: Some(key), base, clock }
    }

    fn tasks(&self, job: Option<&str>) -> Url {
        match job {
            Some(job) => endpoint_url(&self.base, &format!("/contents/generations/tasks/{job}")),
            None => endpoint_url(&self.base, "/contents/generations/tasks"),
        }
    }

    async fn send(&self, http: &demi_provider_common::ProviderHttp, ask: Ask) -> MediaAnswer<Bytes> {
        let headers = ark::headers(self.key.as_ref());
        ark::call(&*self.clock)
            .send(http, ask, Holding::Json, |_| std::future::ready(Ok((headers.clone(), None))))
            .await
            .and_then(|answered| Ok(answered.body))
    }

    async fn read_state(&self, http: &demi_provider_common::ProviderHttp, job: &str) -> MediaAnswer<JobState> {
        self.send(http, Ask::get(&self.tasks(Some(job)))).await.and_then(|body| read_state(&body))
    }
}

impl MediaMaker for ArkVideos {
    fn name(&self) -> &str {
        match &self.flavor {
            Flavor::Vendor(vendor) => vendor.name(),
            Flavor::Compatible { name, .. } => name,
        }
    }

    fn vendor(&self) -> Option<MediaVendor> {
        match self.flavor {
            Flavor::Vendor(vendor) => Some(vendor),
            Flavor::Compatible { .. } => None,
        }
    }

    /// The catalog's models of the key's Ark, which lists none to a key; a
    /// compatible entry's added models.
    fn models<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        let models = match &self.flavor {
            Flavor::Vendor(vendor) => catalog_models(catalog, *vendor, MediaKind::Video),
            Flavor::Compatible { models, .. } => models.clone(),
        };
        Box::pin(std::future::ready(Ok(models)))
    }
}

impl VideoMaker for ArkVideos {
    fn shape(&self) -> VideoShape {
        VideoShape::ArkTasks
    }

    fn ask<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, _: &'a demi_provider_common::media_catalog::Catalog, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(async move {
            let body = Bytes::from(encode(&request));
            self.send(http, Ask::post_json(&self.tasks(None), body)).await.and_then(|body| read_asked(&body))
        })
    }

    fn state<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: demi_provider_common::videos::JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState>>  {
        let job = job.id;
        Box::pin(self.read_state(http, job))
    }

    /// The video's address is temporary and takes no header.
    fn download<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, done: &'a Done) -> BoxFuture<'a, MediaAnswer<Bytes>> {
        Box::pin(videos::download("Ark", &*self.clock, http, &done.url, &self.base, HeaderMap::new()))
    }

    /// Deletes the task while it is still queued; a running one runs to its
    /// end, which Ark cannot stop, and a finished one is left as it is.
    fn cancel<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: demi_provider_common::videos::JobRef<'a>) -> Option<BoxFuture<'a, MediaAnswer<()>>>  {
        let job = job.id;
        Some(Box::pin(async move {
            let read = self.read_state(http, job).await;
            match read.result {
                Ok(JobState::Queued) => self.send(http, Ask::delete(&self.tasks(Some(job)))).await.and_then(|_| Ok(())),
                Ok(_) => MediaAnswer { result: Ok(()), observed: read.observed },
                Err(failure) => MediaAnswer { result: Err(failure), observed: read.observed },
            }
        }))
    }
}

/// The video role of a compatible entry whose video shape is Ark Tasks:
/// Ark's requests under its `base`, with its `key` when it has one,
/// offering the `models` its user added; `name` is the entry's, which its
/// failures name.
pub fn compatible_videos(
    name: String,
    key: Option<Secret>,
    base: &Url,
    models: Vec<MediaModelInfo>,
    clock: Arc<dyn Clock>,
) -> Arc<dyn VideoMaker> {
    Arc::new(ArkVideos { flavor: Flavor::Compatible { name, models }, key, base: base.clone(), clock })
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
fn read_state(body: &[u8]) -> Result<JobState, MediaFailure> {
    let state: State =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its task's state could not be read: {error}")))?;
    match state.status.as_str() {
        "queued" => Ok(JobState::Queued),
        "running" => Ok(JobState::Running { progress: None }),
        "failed" => match state.error {
            // Ark's message quotes the request ID its support asks for.
            Some(TaskError { code, message }) => {
                let id = request_id_in(&message);
                Ok(JobState::Failed { message, trace: VendorTrace { code, id } })
            }
            None => Ok(JobState::Failed { message: "the task failed".to_owned(), trace: VendorTrace::default() }),
        },
        "cancelled" => Ok(JobState::Failed { message: "the task was cancelled".to_owned(), trace: Default::default() }),
        "expired" => Ok(JobState::Expired),
        "succeeded" => {
            let url = state
                .content
                .and_then(|content| content.video_url)
                .filter(|url| !url.is_empty())
                .ok_or_else(|| unreadable("its finished task named no video".to_owned()))?;
            let url = Url::parse(&url).map_err(|error| unreadable(format!("its video's address could not be read: {error}")))?;
            // Ark bills a video by the tokens it made; it reads no input.
            let usage = state.usage.and_then(|usage| usage.completion_tokens).map(|output_tokens| TokenUsage {
                input_tokens: 0,
                output_tokens,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            });
            Ok(JobState::Done(Done { url, cost: None, usage }))
        }
        other => Err(unreadable(format!("its task's state \"{other}\" is not one it documents"))),
    }
}
