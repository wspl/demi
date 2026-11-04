//! The video role of a Gemini API key, Gemini Omni's.
//!
//! A job is an interaction in the background (`crate::interactions`,
//! [Gemini Omni](https://ai.google.dev/gemini-api/docs/omni)): its video
//! comes at an address (`delivery: "uri"`), which is downloaded with the key,
//! and its tokens by kind, which the catalog's rates price.
//!
//! What a request sends:
//!
//! - An image, and a video no kept job made, goes through the Files API
//!   (`crate::files`), and the request names the file.
//! - An image's role is declared at the head of the prompt, in the form
//!   Google's guide gives, such as `[# Sources <FIRST_FRAME>@Image1]`: the
//!   API has no other field for it, and Omni otherwise guesses each role.
//! - A length is `response_format.duration`, such as `"4s"`, as Google's own
//!   Omni client sends it; the API reference does not list the field.
//! - An edit or an extension of a video Google keeps continues its job by
//!   `previous_interaction_id`, in the background as well; of a file, names
//!   it with the change as `video_config.task`, which Google refuses beside a
//!   kept job. A kept job Google no longer has is answered `not_found`.
//!   Continuing a kept job is the one way to change a video where Google
//!   refuses uploaded videos, in the European Economic Area, Switzerland and
//!   the United Kingdom, and to extend one past 10 seconds.
//! - The models are those Google lists for the key that the catalog has as
//!   video models: Google still lists models it shut down.

use bytes::Bytes;
use demi_provider_common::{
    media::{MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, unreadable, within_limit},
    media_catalog::{Catalog, Modality, PriceUnit, Tokens},
    videos::{self, Change, Done, DoneVideo, JobRef, JobState, SourceVideo, VideoMaker, VideoRequest},
};
use demi_shared_types::VideoResolution;
use futures_util::future::BoxFuture;
use reqwest::Url;
use serde::Serialize;

use crate::images::{GeminiKey, LABEL};
use crate::interactions::{self, Standing, Usage};

/// The catalog's vendor of Google's models.
const VENDOR: &str = "google";

/// A Gemini API key's video role.
pub(crate) struct KeyVideos(pub(crate) GeminiKey);

/// The models that list `generateContent`, Omni's method, among which the
/// catalog's video models are kept.
fn generates(_: &[String], methods: &[String]) -> bool {
    methods.iter().any(|method| method == "generateContent")
}

impl MediaMaker for KeyVideos {
    fn name(&self) -> &str {
        LABEL
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(MediaVendor::Google)
    }

    fn models<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(async move {
            let listed = self.0.list(http, generates).await?;
            Ok(listed
                .into_iter()
                .filter(|model| catalog.model(VENDOR, &model.id).is_some_and(|known| catalog.video(known).is_some()))
                .collect())
        })
    }
}

impl VideoMaker for KeyVideos {
    fn changes(&self) -> Vec<Change> {
        vec![Change::Edit, Change::Extend]
    }

    fn continues_jobs(&self) -> bool {
        true
    }

    fn ask<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, _: &'a Catalog, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(async move {
            let key = &self.0;
            let kept = matches!(&request.source, Some(source) if matches!(source.video, SourceVideo::Job(_)));
            let body = match encode(key, http, &request).await {
                Ok(body) => body,
                Err(failure) => return MediaAnswer { result: Err(failure), observed: Vec::new() },
            };
            let body = serde_json::to_value(&body).expect("a video's body serializes");
            let asked = interactions::create(key, http, body).await;
            MediaAnswer { result: asked.result.map_err(|failure| gone(failure, kept)), observed: asked.observed }
        })
    }

    fn state<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState>> {
        Box::pin(async move {
            let read = interactions::get(&self.0, http, job.id).await;
            read.and_then(|interaction| match interaction.standing() {
                Standing::Pending { running: false } => Ok(JobState::Queued),
                Standing::Pending { running: true } => Ok(JobState::Running { progress: None }),
                Standing::Failed(trace) => Ok(JobState::Failed { reason: None, trace }),
                Standing::Completed => {
                    // A model that answers in words instead declined the
                    // video, and says why.
                    let Some(video) = interaction.outputs().rev().find(|content| content.kind == "video") else {
                        return match interaction.said() {
                            Some(said) => Ok(JobState::Failed { reason: Some(said), trace: Default::default() }),
                            None => Err(unreadable("its finished job sent no video".to_owned())),
                        };
                    };
                    let video = match (&video.data, &video.uri) {
                        (Some(data), _) => DoneVideo::Sent(within_limit(data.clone().into_bytes(), "video")?),
                        (None, Some(uri)) => DoneVideo::At(media_of(uri)?),
                        (None, None) => return Err(unreadable("its video came without its bytes or an address".to_owned())),
                    };
                    let usage = interaction.usage.unwrap_or_default();
                    let price = job.catalog.model(VENDOR, job.model).and_then(|known| job.catalog.price(known));
                    let cost = price.and_then(|price| price.of_tokens(None, &priced(&usage)));
                    Ok(JobState::Done(Done { video, cost, usage: Some(usage.counts()) }))
                }
            })
        })
    }

    /// The file's address is the key's own endpoint's, and its redirect to
    /// storage gets no key.
    fn download<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, url: &'a Url) -> BoxFuture<'a, MediaAnswer<Bytes>> {
        Box::pin(videos::download(LABEL, &*self.0.clock, http, url, self.0.base(), self.0.headers()))
    }
}

/// The address of a file's bytes: its `uri`, asked for its media rather
/// than its description.
fn media_of(uri: &str) -> Result<Url, MediaFailure> {
    let mut url = Url::parse(uri).map_err(|error| unreadable(format!("its video's address could not be read: {error}")))?;
    if !url.query_pairs().any(|(name, _)| name == "alt") {
        url.query_pairs_mut().append_pair("alt", "media");
    }
    Ok(url)
}

/// A refusal of a request that continues a kept job, read as the job's
/// being gone when Google answers `not_found`.
fn gone(failure: MediaFailure, kept: bool) -> MediaFailure {
    match failure {
        MediaFailure::Refused { trace, .. } if kept && trace.code.as_deref() == Some("not_found") => {
            MediaFailure::SourceGone { trace }
        }
        failure => failure,
    }
}

/// The counts as the catalog's rates price them: its input, its video and
/// the rest of its output with its thinking, which Google prices as text.
fn priced(usage: &Usage) -> [Tokens; 3] {
    let video: u64 = usage.output_tokens_by_modality.iter().filter(|count| count.modality == "video").map(|count| count.tokens).sum();
    let text = usage.total_output_tokens.saturating_sub(video) + usage.total_thought_tokens;
    [
        Tokens { unit: PriceUnit::MillionInputTokens, modality: None, count: usage.total_input_tokens },
        Tokens { unit: PriceUnit::MillionOutputTokens, modality: Some(Modality::Video), count: video },
        Tokens { unit: PriceUnit::MillionOutputTokens, modality: Some(Modality::Text), count: text },
    ]
}

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    input: Vec<Item>,
    response_format: Format,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_interaction_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Item {
    Text { text: String },
    /// A file of the Files API, by its address.
    Image { uri: String, mime_type: &'static str },
    Video { uri: String, mime_type: &'static str },
}

/// The answer's video, at an address.
#[derive(Serialize)]
struct Format {
    #[serde(rename = "type")]
    kind: &'static str,
    delivery: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<String>,
    /// Seconds, as `4s`.
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<String>,
}

#[derive(Serialize)]
struct GenerationConfig {
    video_config: VideoConfig,
}

#[derive(Serialize)]
struct VideoConfig {
    task: &'static str,
}

/// The ask: the source video, the images in their order, each uploaded,
/// then the prompt with their roles declared; a kept job's id instead of
/// its video.
async fn encode<'a>(
    key: &GeminiKey,
    http: &demi_provider_common::ProviderHttp,
    request: &'a VideoRequest,
) -> Result<Body<'a>, MediaFailure> {
    let mut input = Vec::new();
    let mut previous = None;
    let mut task = None;
    if let Some(source) = &request.source {
        match &source.video {
            SourceVideo::File(bytes) => {
                let uri = key.upload(http, bytes.clone(), "video/mp4").await?;
                input.push(Item::Video { uri, mime_type: "video/mp4" });
                task = Some(match source.change {
                    Change::Edit => "edit",
                    Change::Extend => "extend",
                });
            }
            SourceVideo::Job(id) => previous = Some(id.as_str()),
        }
    }
    for image in request.image.iter().chain(&request.last_frame).chain(&request.references) {
        let uri = key.upload(http, image.bytes.clone(), image.media_type).await?;
        input.push(Item::Image { uri, mime_type: image.media_type });
    }
    input.push(Item::Text { text: prompt(request) });
    Ok(Body {
        model: &request.model,
        input,
        response_format: Format {
            kind: "video",
            delivery: "uri",
            aspect_ratio: request.aspect.map(|aspect| aspect.to_string()),
            resolution: request.resolution.map(VideoResolution::lowercase),
            duration: request.duration.map(|seconds| format!("{seconds}s")),
        },
        previous_interaction_id: previous,
        generation_config: task.map(|task| GenerationConfig { video_config: VideoConfig { task } }),
    })
}

/// The prompt, after the roles of its images, numbered from 1 in the order
/// `encode` sends them: `[# Sources <FIRST_FRAME>@Image1 <LAST_FRAME>@Image2]
/// [# References <IMAGE_REF_0>@Image3]`.
fn prompt(request: &VideoRequest) -> String {
    let mut number = 0;
    let mut next = || {
        number += 1;
        number
    };
    let mut sources = Vec::new();
    if request.image.is_some() {
        sources.push(format!("<FIRST_FRAME>@Image{}", next()));
    }
    if request.last_frame.is_some() {
        sources.push(format!("<LAST_FRAME>@Image{}", next()));
    }
    let references: Vec<String> =
        (0..request.references.len()).map(|index| format!("<IMAGE_REF_{index}>@Image{}", next())).collect();
    let mut heads = Vec::new();
    if !sources.is_empty() {
        heads.push(format!("[# Sources {}]", sources.join(" ")));
    }
    if !references.is_empty() {
        heads.push(format!("[# References {}]", references.join(" ")));
    }
    heads.push(request.prompt.clone());
    heads.join(" ")
}
