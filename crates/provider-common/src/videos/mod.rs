//! The video role of a provider.
//!
//! A provider that makes videos lists its video models and runs one job for its
//! account or key as one stream of its steps: the job's id, then each change of
//! its state until its end, whose video the role downloads. A whole job is one
//! call, so it crosses a channel as one (`crate::media::channel`). Most vendors
//! answer a job's ask with its id and are read until it ends ([`PolledVideos`],
//! [`polled`]); one that streams the job's states, as Gemini Omni does, answers
//! them as they come. The backend chooses the entry and the account, paces the
//! reads and bounds the job.
//!
//! A maker sends every option a request gives, each in its field. What a model
//! takes is its catalog entry's to say: the call checks a request against it
//! before any maker sees it ([`untaken`]).
//!
//! What every vendor's request has in common is in [`crate::media`]. The xAI
//! Videos shape, which a Grok subscription and an xAI key share, is in
//! [`xai`].

pub mod xai;

use std::time::Duration;

use bytes::Bytes;
use demi_shared_types::{Clock, Cost, TokenUsage, VendorTrace, VideoResolution};
use futures_util::future::BoxFuture;
use futures_util::stream::{self, BoxStream, StreamExt as _};
use http::HeaderMap;
use reqwest::Url;
use serde::Serialize;

use crate::images::{Aspect, InputImage};
use crate::media::{ANSWER_LIMIT, Ask, MediaAnswer, MediaCall, MediaCredential, MediaMaker, follow};
use crate::media_catalog::{Catalog, VideoCapabilities};
use crate::read_http_failure;

/// An item of a video ask's `content`, as Ark's tasks and MiniMax's v2
/// take it: the prompt, or an image in its role.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text { text: String },
    ImageUrl { image_url: ImageUrl, role: &'static str },
}

#[derive(Debug, Serialize)]
pub struct ImageUrl {
    pub url: String,
}

/// `request`'s `content`: its prompt, then its first frame, its last frame
/// and its references, each image as a data URL in its role.
pub fn content(request: &VideoRequest) -> Vec<Content> {
    let image = |image: &InputImage, role| Content::ImageUrl { image_url: ImageUrl { url: image.data_url() }, role };
    let mut content = vec![Content::Text { text: request.prompt.clone() }];
    content.extend(request.image.iter().map(|first| image(first, "first_frame")));
    content.extend(request.last_frame.iter().map(|last| image(last, "last_frame")));
    content.extend(request.references.iter().map(|reference| image(reference, "reference_image")));
    content
}

/// A provider's video role.
pub trait VideoMaker: MediaMaker {
    /// The job `request` describes, as the vendor tells it: its id first,
    /// then each state it reads or sends, the last of them its end;
    /// `catalog` says what the model takes, and a vendor that is read is
    /// read every `every`.
    fn run<'a>(
        &'a self,
        http: &'a crate::ProviderHttp,
        catalog: &'a Catalog,
        request: VideoRequest,
        every: Duration,
    ) -> BoxStream<'a, MediaAnswer<JobStep>>;

    /// The changes of a video its request has a field for; none for a
    /// vendor that only makes new ones.
    fn changes(&self) -> Vec<Change> {
        Vec::new()
    }

    /// Whether it continues a job it made by the job's id
    /// ([`SourceVideo::Job`]), the vendor keeping the video.
    fn continues_jobs(&self) -> bool {
        false
    }

    /// Downloads the video a finished job names at `url`, with the entry's
    /// key where the vendor needs it.
    fn download<'a>(&'a self, http: &'a crate::ProviderHttp, url: &'a Url) -> BoxFuture<'a, MediaAnswer<Bytes>>;

    /// Cancels the job `job`, for a vendor that can cancel one; none
    /// elsewhere, where the job runs to its end and is paid for.
    fn cancel<'a>(&'a self, _http: &'a crate::ProviderHttp, _job: JobRef<'a>) -> Option<BoxFuture<'a, MediaAnswer<()>>> {
        None
    }
}

/// The role of a vendor whose job is asked once and then read until it
/// ends, which [`polled`] makes its [`VideoMaker::run`] of.
pub trait PolledVideos: VideoMaker {
    /// Asks for the video `request` describes and answers the job's id;
    /// `catalog` says what the model takes.
    fn ask<'a>(&'a self, http: &'a crate::ProviderHttp, catalog: &'a Catalog, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>>;

    /// Reads the state of the job `job` once.
    fn state<'a>(&'a self, http: &'a crate::ProviderHttp, job: JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState>>;
}

/// One step of a job as [`VideoMaker::run`] tells it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum JobStep {
    /// The vendor took the job, under its id.
    Asked(String),
    /// A state of the job; it ends with [`JobState::Done`],
    /// [`JobState::Failed`] or [`JobState::Expired`].
    State(JobState),
}

/// `maker`'s job as its ask and its reads every `every` tell it, the reads
/// ending at the job's end or the first failure.
pub fn polled<'a, M: PolledVideos + ?Sized>(
    maker: &'a M,
    http: &'a crate::ProviderHttp,
    catalog: &'a Catalog,
    request: VideoRequest,
    every: Duration,
) -> BoxStream<'a, MediaAnswer<JobStep>> {
    let model = request.model.clone();
    let asked = stream::once(maker.ask(http, catalog, request));
    asked
        .flat_map(move |answer| {
            let id = answer.result.as_ref().ok().cloned();
            let first = stream::once(std::future::ready(answer.and_then(|id| Ok(JobStep::Asked(id)))));
            let reads = match id {
                Some(id) => {
                    let model = model.clone();
                    stream::unfold(Some(id), move |id| {
                        let model = model.clone();
                        async move {
                            let id = id?;
                            tokio::time::sleep(every).await;
                            let answer = maker.state(http, JobRef { id: &id, model: &model, catalog }).await;
                            let ended = !matches!(answer.result, Ok(JobState::Queued | JobState::Running { .. }));
                            let next = (!ended).then_some(id);
                            Some((answer.and_then(|state| Ok(JobStep::State(state))), next))
                        }
                    })
                    .boxed()
                }
                None => stream::empty().boxed(),
            };
            first.chain(reads)
        })
        .boxed()
}

/// What `demi video edit` and `demi video extend` ask of a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum Change {
    /// The same scene changed as the prompt says, its length and shape
    /// kept.
    Edit,
    /// The video followed by its continuation, as one video.
    Extend,
}

/// The video an edit or an extension changes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct VideoSource {
    pub change: Change,
    pub video: SourceVideo,
}

/// How the source goes to the vendor.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum SourceVideo {
    /// An MP4's bytes.
    File(Bytes),
    /// The vendor's id of the job that made it, which the vendor keeps; a
    /// vendor that no longer does fails the job with
    /// [`MediaFailure::SourceGone`](crate::media::MediaFailure::SourceGone).
    /// It never crosses a channel: a channel's maker continues no job
    /// ([`VideoMaker::continues_jobs`]), whose id names the vendor.
    #[serde(skip)]
    Job(String),
}

/// A job a vendor took: its id at the vendor, and the model it makes as
/// `catalog` states it, by which a vendor whose models take different
/// requests, as MiniMax's do, reads its state.
#[derive(Debug, Clone, Copy)]
pub struct JobRef<'a> {
    pub id: &'a str,
    pub model: &'a str,
    pub catalog: &'a Catalog,
}

/// One video to make; an option left out is the vendor's default.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct VideoRequest {
    pub model: String,
    pub prompt: String,
    /// The first frame.
    pub image: Option<InputImage>,
    /// The last frame, with a first.
    pub last_frame: Option<InputImage>,
    /// Images the video keeps to, such as a character or a product.
    pub references: Vec<InputImage>,
    /// Seconds.
    pub duration: Option<u32>,
    pub aspect: Option<Aspect>,
    pub resolution: Option<VideoResolution>,
    /// Whether the model makes sound.
    pub audio: Option<bool>,
    pub seed: Option<u64>,
    /// The video an edit or an extension changes; none for a new one.
    pub source: Option<VideoSource>,
}

/// What `request` asks that the model's catalog entry `takes` says it does
/// not take, in a call's words, such as `takes --duration 3 to 10`, the first
/// in the order of the options; none when the entry excludes nothing it asks.
/// A fact the entry does not state excludes nothing.
pub fn untaken(takes: &VideoCapabilities, request: &VideoRequest) -> Option<String> {
    let given_image = request.image.is_some();
    if given_image && takes.first_frame == Some(false) {
        return Some("takes no --image".to_owned());
    }
    if request.last_frame.is_some() && takes.last_frame == Some(false) {
        return Some("takes no --last-frame".to_owned());
    }
    match takes.max_reference_images {
        Some(0) if !request.references.is_empty() => return Some("takes no --reference".to_owned()),
        Some(most) if request.references.len() > most as usize => {
            return Some(format!("takes at most {most} --reference"));
        }
        _ => {}
    }
    if let Some(asked) = request.duration
        && let Some(lengths) = lengths(takes, asked)
    {
        return Some(format!("takes --duration {lengths}"));
    }
    if let Some(aspect) = request.aspect {
        if given_image && takes.aspect_with_first_frame == Some(false) {
            return Some("takes no --aspect with --image: the image sets the shape".to_owned());
        }
        if let Some(ratios) = &takes.aspect_ratios
            && !ratios.iter().filter_map(|ratio| Aspect::parse(ratio)).any(|ratio| ratio.same_shape(aspect))
        {
            return Some(match ratios.is_empty() {
                true => "takes no --aspect".to_owned(),
                false => format!("takes --aspect {}", either(ratios)),
            });
        }
    }
    if let Some(asked) = request.resolution
        && let Some(resolutions) = &takes.resolutions
        && !resolutions.iter().any(|named| VideoResolution::try_from(named.clone()).is_ok_and(|named| named == asked))
    {
        return Some(match resolutions.is_empty() {
            true => "takes no --resolution".to_owned(),
            false => format!("takes --resolution {}", either(resolutions)),
        });
    }
    match request.audio {
        Some(true) if takes.audio == Some(false) => return Some("makes no sound: it takes no --audio".to_owned()),
        // A silent model honors --no-audio without a field for it.
        Some(false) if takes.audio_off == Some(false) && takes.audio != Some(false) => {
            return Some("always makes sound: it takes no --no-audio".to_owned());
        }
        _ => {}
    }
    if request.seed.is_some() && takes.seed == Some(false) {
        return Some("takes no --seed".to_owned());
    }
    None
}

/// The lengths `takes` states, as a call's words say them, when `asked` is
/// none of them: `6 or 10`, `3 to 10`, `at least 3`.
fn lengths(takes: &VideoCapabilities, asked: u32) -> Option<String> {
    let seconds = |value: f64| {
        let text = format!("{value:.1}");
        text.strip_suffix(".0").unwrap_or(&text).to_owned()
    };
    if let Some(durations) = &takes.durations {
        if durations.contains(&asked) {
            return None;
        }
        let named: Vec<String> = durations.iter().map(u32::to_string).collect();
        return Some(either(&named));
    }
    let range = takes.duration_range?;
    let asked = f64::from(asked);
    match (range.min, range.max) {
        (Some(min), Some(max)) => (asked < min || asked > max).then(|| format!("{} to {}", seconds(min), seconds(max))),
        (Some(min), None) => (asked < min).then(|| format!("at least {}", seconds(min))),
        (None, Some(max)) => (asked > max).then(|| format!("at most {}", seconds(max))),
        (None, None) => None,
    }
}

/// `a`, `a or b`, `a, b or c`.
fn either(values: &[String]) -> String {
    match values {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// A job's state as one read of it says.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum JobState {
    /// Waiting for the vendor to start it.
    Queued,
    /// Being made, with how far along when the vendor says.
    Running { progress: Option<u8> },
    /// Made, and where the video is.
    Done(Done),
    /// The vendor failed the video; `reason` is Demi's, for a failure it
    /// read out of an answer, such as a video withheld, and `trace` what
    /// the vendor said. Neither crosses a relay.
    Failed {
        #[serde(skip)]
        reason: Option<String>,
        #[serde(skip)]
        trace: VendorTrace,
    },
    /// The job expired at the vendor before it ran.
    Expired,
}

/// A finished job: its video and what the vendor said it cost.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Done {
    pub video: DoneVideo,
    /// The cost the vendor reported, in its documented unit, or the
    /// tokens it reported priced at its published rates; none otherwise.
    pub cost: Option<Cost>,
    /// The counts the vendor reported; none when it reported none.
    pub usage: Option<TokenUsage>,
}

/// A finished job's video, as the vendor gives it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum DoneVideo {
    /// At an address, which [`VideoMaker::download`] downloads. It never
    /// crosses a channel, which sends the video instead
    /// ([`crate::media::channel::serve`]): the address names the vendor.
    #[serde(skip)]
    At(Url),
    /// Within the answer, as a channel sends it, or as Gemini Omni sends a
    /// video it keeps for the job's continuation.
    Sent(Bytes),
}

/// Downloads `url` as Demi's one download
/// (`crates/shared-artifacts/src/download.rs`): resumed after a break or a
/// stall, without an overall deadline.
///
/// The entry's `key` headers go only to an address of the origin of the entry's
/// `endpoint`, never to another host's, a storage bucket's or a redirect's to
/// another origin, which each get the request without them ([`follow`]).
///
/// The first answer is read here, so a refusal is told in the vendor's words
/// under `label`. An address is no account's, so nothing is renewed and no
/// limit is read from it. A video over [`ANSWER_LIMIT`] fails as it arrives.
pub async fn download(
    label: &'static str,
    clock: &dyn Clock,
    http: &crate::ProviderHttp,
    url: &Url,
    endpoint: &Url,
    key: HeaderMap,
) -> MediaAnswer<Bytes> {
    let call = MediaCall {
        label,
        credential: MediaCredential::Key { refused: |_, _| false },
        reader: read_http_failure,
        used_up: |failure| failure,
        clock,
        quota: None,
    };
    let result = async {
        let first = crate::media::within_deadline(async {
            let response = follow(http, &Ask::get(url), &key, endpoint).await?;
            if !response.status().is_success() {
                return Err(call.refusal(response).await);
            }
            Ok(response)
        })
        .await?;
        let http = http.without_redirects().map_err(crate::media::transport)?;
        let origin = endpoint.origin();
        let headers = move |hop: &Url| if hop.origin() == origin { key.clone() } else { HeaderMap::new() };
        let request = demi_shared_artifacts::Download {
            limit: ANSWER_LIMIT as u64,
            redirects: crate::client::REDIRECTS,
            headers: &headers,
            first: Some(first),
            // The client decodes the codings the build's reqwest knows, so
            // the first answer's bytes may count none the vendor sent.
            first_decoded: true,
            ..demi_shared_artifacts::Download::new(url.as_str())
        };
        // A video call ends with its job, which drops this future: nothing
        // else cancels its download.
        let cancel = tokio_util::sync::CancellationToken::new();
        let downloaded = demi_shared_artifacts::download_bytes(&http, request, &cancel)
            .await
            .and_then(|(received, video)| received.plain().map(|()| video));
        match downloaded {
            Ok(video) => Ok(Bytes::from(video)),
            Err(demi_shared_artifacts::Error::TooLarge { .. }) => Err(crate::media::too_large("video")),
            Err(error) => Err(crate::media::unreadable(format!("its video did not download: {error}"))),
        }
    }
    .await;
    MediaAnswer { result, observed: Vec::new() }
}
