//! How xAI's image API, a Grok subscription's and an xAI key's alike,
//! refuses a shape, and how an xAI key's image is a job.
//!
//! An image asked with `deferred: true` is a job: the ask answers a
//! `request_id`, and `GET …/images/<request_id>` answers 202 with `status:
//! pending` while it runs, then 200 with `done`, the image's temporary
//! address and its cost in ticks, as a video's job does
//! (`crate::videos::xai`). A deferred image comes only as an address.

use demi_shared_types::{Cost, Currency, VendorTrace};
use reqwest::Url;
use serde::Deserialize;

use crate::media::jobs::JobState;
use crate::media::{MediaAnswer, MediaFailure, unreadable};

/// Ticks in one US dollar.
const TICKS_PER_USD: f64 = 1e10;

/// A cost xAI reports in ticks, ten-billionths of a dollar ([Cost
/// Tracking](https://docs.x.ai/developers/cost-tracking)).
pub(crate) fn ticks(ticks: u64) -> Cost {
    Cost { amount: ticks as f64 / TICKS_PER_USD, currency: Currency::Usd }
}

/// A finished image's address and what it cost.
pub struct DeferredImage {
    pub url: Url,
    pub cost: Option<Cost>,
}

#[derive(Deserialize)]
struct State {
    status: String,
    #[serde(default)]
    data: Vec<Datum>,
    #[serde(default)]
    error: Option<Said>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Datum {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct Said {
    message: String,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    cost_in_usd_ticks: Option<u64>,
}

/// The state one read of a deferred image answered, from its body: 202
/// and 200 alike carry `status`.
pub fn read_state(body: &[u8]) -> Result<JobState<DeferredImage>, MediaFailure> {
    let state: State =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its job's state could not be read: {error}")))?;
    match state.status.as_str() {
        "pending" => Ok(JobState::Running { progress: None }),
        "failed" => Ok(JobState::Failed { reason: None, trace: VendorTrace::default().with_words(state.error.map(|said| said.message)) }),
        "expired" => Ok(JobState::Expired),
        "done" => {
            let Some(url) = state.data.into_iter().find_map(|datum| datum.url.filter(|url| !url.is_empty())) else {
                return Ok(JobState::Failed { reason: Some("its finished job named no image".to_owned()), trace: VendorTrace::default() });
            };
            let url = Url::parse(&url).map_err(|error| unreadable(format!("its image's address could not be read: {error}")))?;
            let cost = state.usage.and_then(|usage| usage.cost_in_usd_ticks).map(ticks);
            Ok(JobState::Done(DeferredImage { url, cost }))
        }
        other => Err(unreadable(format!("its job's state \"{other}\" is not one it documents"))),
    }
}

/// The shapes xAI's image API takes, as its refusal of `aspect_ratio` lists
/// them: ``aspect_ratio: unknown variant `4:5`, expected one of `1:1`, `3:4`,
/// …, `auto` at line 1 column 304``.
///
/// The whole list is read, each entry quoted and the last after `, ` or ` or `.
/// Only the W:H shapes are kept, as `auto` is no shape `--aspect` asks for.
/// None for any other refusal or a list not read whole, which is then told in
/// xAI's own words.
fn shapes_taken(message: &str) -> Option<Vec<String>> {
    let (_, rest) = message.split_once("aspect_ratio: unknown variant `")?;
    let (_, mut rest) = rest.split_once("`, expected one of ")?;
    let mut takes = Vec::new();
    loop {
        let (entry, after) = rest.strip_prefix('`')?.split_once('`')?;
        if entry.contains(':') {
            takes.push(entry.to_owned());
        } else if entry != "auto" {
            return None;
        }
        match after.strip_prefix(", ").or_else(|| after.strip_prefix(" or ")) {
            Some(next) => rest = next,
            // The list ends where no separator follows: at the message's
            // end or its position, `at line …`.
            None if after.is_empty() || after.starts_with(" at ") => break,
            None => return None,
        }
    }
    (!takes.is_empty()).then_some(takes)
}

/// `answer` with a refusal of the shape asked for told as the shapes xAI
/// named, so a shape xAI adds needs no change; xAI's words stay with it.
pub fn with_shapes<T>(mut answer: MediaAnswer<T>) -> MediaAnswer<T> {
    if let Err(MediaFailure::Refused { code, trace, .. }) = &answer.result
        && let Some(takes) = trace.words.as_deref().and_then(shapes_taken)
    {
        answer.result = Err(MediaFailure::Shape { takes, code: code.clone(), trace: trace.clone() });
    }
    answer
}
