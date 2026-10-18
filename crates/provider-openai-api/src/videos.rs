//! The video role of an xAI key and of a compatible entry with a video
//! shape (`videos.md` § Vendors): the xAI Videos shape under the entry's
//! base URL for either, and for a compatible API the OpenAI Videos or the
//! OpenRouter Videos shape it chose, each with the key as a bearer token.
//! An xAI key lists the video models xAI lists for it; a compatible entry
//! offers the video models its user added.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    FormBody, Secret, endpoint_url,
    images::{Aspect, InputImage},
    json_body,
    media::{Ask, Holding, MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, unreadable},
    media_catalog::Catalog,
    videos::{self, Done, JobState, Resolution, VideoMaker, VideoRequest, xai},
};
use demi_shared_types::{Clock, Cost, Currency, VideoShape};
use futures_util::future::BoxFuture;
use reqwest::{
    Url,
    header::{AUTHORIZATION, HeaderMap},
};
use serde::{Deserialize, Serialize};

use crate::images::{key_call, xai_listing};

/// Whose video API an entry's requests go to.
enum Flavor {
    Xai,
    /// A compatible endpoint, which the user knows by the entry's name,
    /// whose models are the ones the user added and whose shape the user
    /// chose.
    Compatible { name: String, models: Vec<MediaModelInfo>, shape: VideoShape },
}

/// The video role of a key or a compatible endpoint.
pub(crate) struct KeyVideos {
    flavor: Flavor,
    /// None for a compatible endpoint that takes none.
    key: Option<Secret>,
    base: Url,
    clock: Arc<dyn Clock>,
}

impl KeyVideos {
    /// The role of an xAI `key` under the API `base`.
    pub(crate) fn xai(key: Secret, base: &Url, clock: Arc<dyn Clock>) -> Self {
        Self { flavor: Flavor::Xai, key: Some(key), base: base.clone(), clock }
    }

    fn label(&self) -> &str {
        match &self.flavor {
            Flavor::Xai => MediaVendor::Xai.name(),
            Flavor::Compatible { name, .. } => name,
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(key) = &self.key {
            headers.insert(AUTHORIZATION, key.bearer());
        }
        headers
    }

    fn url(&self, path: &str) -> Url {
        endpoint_url(&self.base, path)
    }

    /// Sends what `request` builds with the key's headers and answers the
    /// vendor's success.
    async fn send(&self, http: &demi_provider_common::ProviderHttp, ask: Ask) -> MediaAnswer<Bytes> {
        let headers = self.headers();
        key_call(self.shape_is_xai(), &*self.clock)
            .send(http, ask, Holding::Json, |_| std::future::ready(Ok((headers.clone(), None))))
            .await
            .and_then(|answered| Ok(answered.body))
    }

    fn shape_is_xai(&self) -> bool {
        matches!(self.flavor, Flavor::Xai)
    }
}

impl MediaMaker for KeyVideos {
    fn name(&self) -> &str {
        self.label()
    }

    fn vendor(&self) -> Option<MediaVendor> {
        match self.flavor {
            Flavor::Xai => Some(MediaVendor::Xai),
            Flavor::Compatible { .. } => None,
        }
    }

    /// xAI's listing of the key's video models, read when asked; a
    /// compatible entry's added models.
    fn models<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(async move {
            match &self.flavor {
                Flavor::Compatible { models, .. } => Ok(models.clone()),
                Flavor::Xai => {
                    let url = self.url("/video-generation-models");
                    let text = key_call(true, &*self.clock).get(http, &url, self.headers()).await?;
                    xai_listing(&text)
                }
            }
        })
    }
}

impl VideoMaker for KeyVideos {
    fn shape(&self) -> VideoShape {
        match self.flavor {
            Flavor::Xai => VideoShape::XaiVideos,
            Flavor::Compatible { shape, .. } => shape,
        }
    }

    fn ask<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(async move {
            match self.shape() {
                VideoShape::OpenaiVideos => {
                    let url = self.url("/videos");
                    let (content_type, body) = openai_form(&openai_fields(&request));
                    self.send(http, Ask::post(&url, &content_type, body))
                        .await
                        .and_then(|body| read_id(&body))
                }
                VideoShape::OpenrouterVideos => {
                    let url = self.url("/videos");
                    let body = Bytes::from(openrouter_body(&request));
                    self.send(http, Ask::post_json(&url, body)).await.and_then(|body| read_id(&body))
                }
                VideoShape::Grok | VideoShape::XaiVideos | VideoShape::Veo => {
                    let url = self.url("/videos/generations");
                    let body = xai::encode(&request);
                    self.send(http, Ask::post_json(&url, body))
                        .await
                        .and_then(|body| xai::read_asked(&body))
                }
            }
        })
    }

    fn state<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: &'a str) -> BoxFuture<'a, MediaAnswer<JobState>> {
        Box::pin(async move {
            let url = self.url(&format!("/videos/{job}"));
            let answer = self.send(http, Ask::get(&url)).await;
            match self.shape() {
                VideoShape::OpenaiVideos => {
                    let content = self.url(&format!("/videos/{job}/content"));
                    answer.and_then(|body| read_openai_state(&body, content))
                }
                VideoShape::OpenrouterVideos => answer.and_then(|body| read_openrouter_state(&body)),
                VideoShape::Grok | VideoShape::XaiVideos | VideoShape::Veo => {
                    answer.and_then(|body| xai::read_state(&body))
                }
            }
        })
    }

    /// xAI's address is temporary and takes no header; OpenAI Videos'
    /// content and OpenRouter's addresses take the key, which goes only to
    /// the entry's own endpoint's origin.
    fn download<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, done: &'a Done) -> BoxFuture<'a, MediaAnswer<Bytes>> {
        let headers = match self.shape() {
            VideoShape::OpenaiVideos | VideoShape::OpenrouterVideos => self.headers(),
            VideoShape::Grok | VideoShape::XaiVideos | VideoShape::Veo => HeaderMap::new(),
        };
        let label = if self.shape_is_xai() { "xAI" } else { "OpenAI" };
        Box::pin(videos::download(label, &*self.clock, http, &done.url, &self.base, headers))
    }

    /// OpenAI Videos' servers abort a job its `DELETE` names, as vLLM-Omni
    /// documents; xAI and OpenRouter document no cancellation.
    fn cancel<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: &'a str) -> Option<BoxFuture<'a, MediaAnswer<()>>> {
        if self.shape() != VideoShape::OpenaiVideos {
            return None;
        }
        Some(Box::pin(async move {
            let url = self.url(&format!("/videos/{job}"));
            self.send(http, Ask::delete(&url)).await.and_then(|_| Ok(()))
        }))
    }
}

/// The video role of a compatible entry with a video shape: the shape's
/// requests under its `base`, with its `key` when it has one, offering the
/// `models` its user added; `name` is the entry's, which its failures
/// name.
pub fn compatible_videos(
    name: String,
    shape: VideoShape,
    key: Option<Secret>,
    base: &Url,
    models: Vec<MediaModelInfo>,
    clock: Arc<dyn Clock>,
) -> Arc<dyn VideoMaker> {
    Arc::new(KeyVideos {
        flavor: Flavor::Compatible { name, models, shape },
        key,
        base: base.clone(),
        clock,
    })
}

/// The id of the job an OpenAI Videos or OpenRouter ask answers.
fn read_id(body: &[u8]) -> Result<String, MediaFailure> {
    #[derive(Deserialize)]
    struct Asked {
        id: String,
    }
    let asked: Asked =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    Ok(asked.id)
}

/// The fields of an OpenAI Videos ask: `seconds` as a string, the size of
/// the asked aspect at the asked resolution, and the first frame as
/// `input_reference`.
struct OpenAiFields {
    model: String,
    prompt: String,
    seconds: Option<String>,
    size: Option<String>,
    reference: Option<InputImage>,
}

fn openai_fields(request: &VideoRequest) -> OpenAiFields {
    OpenAiFields {
        model: request.model.clone(),
        prompt: request.prompt.clone(),
        seconds: request.duration.map(|seconds| seconds.to_string()),
        size: openai_size(request.aspect, request.resolution),
        reference: request.image.clone(),
    }
}

/// The size of `aspect` at `resolution`, its shorter side the
/// resolution's, each side a multiple of 16: 1280×720 for 16:9 at 720p.
/// Either alone takes the other's usual value, 16:9 or 720p; neither sends
/// no size, so the server's default stands.
fn openai_size(aspect: Option<Aspect>, resolution: Option<Resolution>) -> Option<String> {
    if aspect.is_none() && resolution.is_none() {
        return None;
    }
    let aspect = aspect.unwrap_or(Aspect { width: 16, height: 9 });
    let short = resolution.unwrap_or(Resolution::P720).short_side();
    let (long, short_part) = if aspect.width >= aspect.height {
        (aspect.width, aspect.height)
    } else {
        (aspect.height, aspect.width)
    };
    let long_side = ((f64::from(short) * f64::from(long) / f64::from(short_part) / 16.0).round() as u32) * 16;
    let (width, height) = aspect.size(long_side);
    Some(format!("{width}x{height}"))
}

/// The ask as `multipart/form-data`, the form OpenAI's reference and
/// vLLM-Omni take: its `Content-Type` and its body, built whole so the ask
/// also reaches a server on a device's exposed port.
fn openai_form(fields: &OpenAiFields) -> (String, Bytes) {
    let mut form = FormBody::new().text("model", &fields.model).text("prompt", &fields.prompt);
    if let Some(seconds) = &fields.seconds {
        form = form.text("seconds", seconds);
    }
    if let Some(size) = &fields.size {
        form = form.text("size", size);
    }
    if let Some(image) = &fields.reference {
        let extension = image.media_type.trim_start_matches("image/");
        form = form.file("input_reference", &format!("reference.{extension}"), image.media_type, &image.bytes);
    }
    form.finish()
}

#[derive(Deserialize)]
struct OpenAiState {
    status: String,
    #[serde(default)]
    progress: Option<f64>,
    #[serde(default)]
    error: Option<VendorError>,
}

#[derive(Deserialize)]
struct VendorError {
    message: String,
}

/// An OpenAI Videos job's state: `queued`, `in_progress`, `completed` with
/// its video at `content`, or `failed`.
fn read_openai_state(body: &[u8], content: Url) -> Result<JobState, MediaFailure> {
    let state: OpenAiState =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its job's state could not be read: {error}")))?;
    let progress = state.progress.map(|progress| progress.clamp(0.0, 100.0) as u8);
    match state.status.as_str() {
        "queued" => Ok(JobState::Queued),
        "in_progress" => Ok(JobState::Running { progress }),
        "completed" => Ok(JobState::Done(Done { url: content, cost: None, usage: None })),
        "failed" => Ok(JobState::Failed {
            message: state.error.map_or_else(|| "the job failed".to_owned(), |error| error.message),
        }),
        other => Err(unreadable(format!("its job's state \"{other}\" is not one it documents"))),
    }
}

#[derive(Serialize)]
struct OpenRouterBody<'a> {
    model: &'a str,
    prompt: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    frame_images: Vec<OpenRouterImage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    input_references: Vec<OpenRouterImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generate_audio: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
}

#[derive(Serialize)]
struct OpenRouterImage {
    #[serde(rename = "type")]
    kind: &'static str,
    image_url: OpenRouterUrl,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_type: Option<&'static str>,
}

#[derive(Serialize)]
struct OpenRouterUrl {
    url: String,
}

fn openrouter_image(image: &InputImage, frame_type: Option<&'static str>) -> OpenRouterImage {
    OpenRouterImage {
        kind: "image_url",
        image_url: OpenRouterUrl { url: image.data_url() },
        frame_type,
    }
}

fn openrouter_body(request: &VideoRequest) -> Vec<u8> {
    let frames = [(request.image.as_ref(), "first_frame"), (request.last_frame.as_ref(), "last_frame")];
    json_body(&OpenRouterBody {
        model: &request.model,
        prompt: &request.prompt,
        frame_images: frames
            .into_iter()
            .filter_map(|(image, frame)| image.map(|image| openrouter_image(image, Some(frame))))
            .collect(),
        input_references: request.references.iter().map(|image| openrouter_image(image, None)).collect(),
        duration: request.duration,
        aspect_ratio: request.aspect.map(|aspect| aspect.to_string()),
        resolution: request.resolution.map(Resolution::as_str),
        generate_audio: request.audio,
        seed: request.seed,
    })
}

#[derive(Deserialize)]
struct OpenRouterState {
    status: String,
    #[serde(default)]
    unsigned_urls: Vec<String>,
    #[serde(default)]
    usage: Option<OpenRouterUsage>,
    #[serde(default)]
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct OpenRouterUsage {
    #[serde(default)]
    cost: Option<f64>,
}

/// An OpenRouter job's state: `pending`, `in_progress`, `completed` with
/// its first address and its cost in dollars, or `failed` with its
/// `error`, text or an object with a message.
fn read_openrouter_state(body: &[u8]) -> Result<JobState, MediaFailure> {
    let state: OpenRouterState =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its job's state could not be read: {error}")))?;
    match state.status.as_str() {
        "pending" => Ok(JobState::Queued),
        "in_progress" => Ok(JobState::Running { progress: None }),
        "completed" => {
            let address = state
                .unsigned_urls
                .first()
                .ok_or_else(|| unreadable("its finished job named no video".to_owned()))?;
            let url = Url::parse(address)
                .map_err(|error| unreadable(format!("its video's address could not be read: {error}")))?;
            let cost = state
                .usage
                .and_then(|usage| usage.cost)
                .map(|amount| Cost { amount, currency: Currency::Usd });
            Ok(JobState::Done(Done { url, cost, usage: None }))
        }
        "failed" => {
            let message = match state.error {
                Some(serde_json::Value::String(text)) => text,
                Some(serde_json::Value::Object(error)) => error
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("the job failed")
                    .to_owned(),
                _ => "the job failed".to_owned(),
            };
            Ok(JobState::Failed { message })
        }
        other => Err(unreadable(format!("its job's state \"{other}\" is not one it documents"))),
    }
}
