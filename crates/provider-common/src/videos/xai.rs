//! The xAI Videos shape, which a Grok subscription's chat proxy, an xAI key and
//! a compatible API speak alike.
//!
//! - `POST …/videos/generations` answers a `request_id`.
//! - `GET …/videos/<request_id>` answers 202 with `status: pending` and its
//!   `progress` while the job runs, then 200 with `done`, `failed` or
//!   `expired`.
//! - A finished video is a temporary address, downloaded without a header.
//! - Its cost is `usage.cost_in_usd_ticks`, ten-billionths of a dollar ([Cost
//!   Tracking](https://docs.x.ai/developers/cost-tracking)).

use bytes::Bytes;
use demi_shared_types::{Cost, Currency, TokenUsage, VideoResolution};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use super::{Done, JobState, VideoRequest};
use crate::json_body;
use crate::media::{MediaFailure, unreadable};

/// Ticks in one US dollar.
const TICKS_PER_USD: f64 = 1e10;

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<Image>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_frame: Option<Image>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reference_images: Vec<Image>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generate_audio: Option<bool>,
}

/// An image as the shape takes one: a `data:` URL.
#[derive(Serialize)]
struct Image {
    url: String,
}

/// The body of the ask: every option the shape has a field for, sent only
/// when given; it has none for a seed.
pub fn encode(request: &VideoRequest) -> Bytes {
    let image = |image: &crate::images::InputImage| Image { url: image.data_url() };
    let body = Body {
        model: &request.model,
        prompt: &request.prompt,
        image: request.image.as_ref().map(image),
        last_frame: request.last_frame.as_ref().map(image),
        reference_images: request.references.iter().map(image).collect(),
        duration: request.duration,
        aspect_ratio: request.aspect.map(|aspect| aspect.to_string()),
        resolution: request.resolution.map(VideoResolution::lowercase),
        generate_audio: request.audio,
    };
    Bytes::from(json_body(&body))
}

/// The job's id in the ask's answer.
pub fn read_asked(body: &[u8]) -> Result<String, MediaFailure> {
    #[derive(Deserialize)]
    struct Asked {
        request_id: String,
    }
    let asked: Asked =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    Ok(asked.request_id)
}

#[derive(Deserialize)]
struct State {
    status: String,
    #[serde(default)]
    progress: Option<u8>,
    #[serde(default)]
    video: Option<Video>,
    #[serde(default)]
    error: Option<Error>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Video {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    respect_moderation: Option<bool>,
}

#[derive(Deserialize)]
struct Error {
    message: String,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    cost_in_usd_ticks: Option<u64>,
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

/// The state one read of the job answered, from its body: 202 and 200
/// alike carry `status`. A finished video the vendor withheld for its
/// moderation has no address, and fails as the vendor's refusal.
pub fn read_state(body: &[u8]) -> Result<JobState, MediaFailure> {
    let state: State =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its job's state could not be read: {error}")))?;
    match state.status.as_str() {
        "pending" => Ok(JobState::Running { progress: state.progress }),
        "failed" => Ok(JobState::Failed {
            message: state.error.map_or_else(|| "the job failed".to_owned(), |error| error.message), trace: Default::default(),
        }),
        "expired" => Ok(JobState::Expired),
        "done" => {
            let video = state.video.ok_or_else(|| unreadable("its finished job named no video".to_owned()))?;
            let Some(url) = video.url.filter(|url| !url.is_empty()) else {
                return Ok(JobState::Failed {
                    message: match video.respect_moderation {
                        Some(false) => "the video was withheld by its moderation".to_owned(),
                        _ => "its finished job named no video".to_owned(),
                    }, trace: Default::default(),
                });
            };
            let url = Url::parse(&url).map_err(|error| unreadable(format!("its video's address could not be read: {error}")))?;
            let usage = state.usage;
            Ok(JobState::Done(Done {
                url,
                cost: usage
                    .as_ref()
                    .and_then(|usage| usage.cost_in_usd_ticks)
                    .map(|ticks| Cost { amount: ticks as f64 / TICKS_PER_USD, currency: Currency::Usd }),
                usage: usage.and_then(|usage| {
                    Some(TokenUsage {
                        input_tokens: usage.input_tokens?,
                        output_tokens: usage.output_tokens?,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                    })
                }),
            }))
        }
        other => Err(unreadable(format!("its job's state \"{other}\" is not one it documents"))),
    }
}
