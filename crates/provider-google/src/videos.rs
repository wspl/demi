//! The video role of a Gemini API key, Gemini Omni's.
//!
//! A job is one streamed interaction
//! ([Gemini Omni](https://ai.google.dev/gemini-api/docs/omni)):
//!
//! 1. `POST …/interactions?alt=sse` with `"stream": true`. Its first event,
//!    `interaction.created`, names the job; Google sends a keep-alive while
//!    the video is made.
//! 2. A `model_output` step's `video` delta holds the video, as base64.
//! 3. `interaction.completed` holds the job's tokens by kind, which the
//!    catalog's rates price.
//!
//! Two choices follow from what Google continues:
//!
//! - The stream, not a job in the background with polling, because Google
//!   fails every continuation of a kept job asked in the background.
//! - The video within the answer, not at an address (`delivery: "uri"`),
//!   because Google continues only a job whose video it sent within its
//!   answer: one it sent at an address it refuses as "HTTP video input".
//!   Continuing a kept job is the one way to change a video where Google
//!   refuses uploaded videos, in the European Economic Area, Switzerland and
//!   the United Kingdom, and to extend one past 10 seconds.
//!
//! What a request sends:
//!
//! - An image, and a video no kept job made, goes through the Files API
//!   (`crate::files`), and the request names the file.
//! - An image's role is declared at the head of the prompt, in the form
//!   Google's guide gives, such as `[# Sources <FIRST_FRAME>@Image1]`: the
//!   API has no other field for it, and Omni otherwise guesses each role.
//! - An edit or an extension of a video Google keeps continues its job by
//!   `previous_interaction_id`; of a file, names it with the change as
//!   `video_config.task`, which Google refuses beside a kept job. A kept
//!   job Google no longer has is answered `not_found`.
//! - The models are those Google lists for the key that the catalog has as
//!   video models: Google still lists models it shut down.

use bytes::Bytes;
use demi_provider_common::{
    endpoint_url, json_body,
    media::{MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, unreadable, within_limit},
    media_catalog::{Catalog, Modality, PriceUnit},
    videos::{
        self, Change, Done, DoneVideo, JobState, JobStep, SourceVideo, VideoMaker, VideoOption, VideoRequest, unsent_of,
    },
};
use demi_shared_types::{Cost, Currency, TokenUsage, VendorTrace, VideoResolution};
use futures_util::future::BoxFuture;
use futures_util::stream::{self, BoxStream, StreamExt as _};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::images::{GeminiKey, LABEL};

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
    /// Omni has no field for a length, sound or a seed, and keeps the
    /// shape of a video it changes.
    fn unsent(&self, _: &Catalog, request: &VideoRequest) -> Vec<VideoOption> {
        let mut lacks = vec![VideoOption::Duration, VideoOption::Audio, VideoOption::Seed];
        if request.source.is_some() {
            lacks.push(VideoOption::Aspect);
        }
        unsent_of(&lacks, request)
    }

    fn shape_name(&self) -> String {
        "Gemini Interactions".to_owned()
    }

    fn changes(&self) -> Vec<Change> {
        vec![Change::Edit, Change::Extend]
    }

    fn continues_jobs(&self) -> bool {
        true
    }

    fn run<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
        request: VideoRequest,
        _: std::time::Duration,
    ) -> BoxStream<'a, MediaAnswer<JobStep>> {
        let key = &self.0;
        let mut url = endpoint_url(key.base(), "/interactions");
        url.query_pairs_mut().append_pair("alt", "sse");
        let rates = Rates::of(catalog, &request.model);
        let kept = matches!(&request.source, Some(source) if matches!(source.video, SourceVideo::Job(_)));
        let opened = async move {
            let body = Bytes::from(json_body(&encode(key, http, &request).await?));
            key.call().post_events(http, &url, body, key.headers()).await
        };
        stream::once(opened)
            .flat_map(move |opened| match opened {
                Ok(events) => read_events(events, rates).boxed(),
                Err(failure) => stream::once(std::future::ready(answer(Err(gone(failure, kept))))).boxed(),
            })
            .boxed()
    }

    /// Omni sends its video within its answer; a video at an address is
    /// downloaded with the key all the same.
    fn download<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, url: &'a reqwest::Url) -> BoxFuture<'a, MediaAnswer<Bytes>> {
        // The file's address is the key's own endpoint's, and its redirect
        // to storage gets no key.
        Box::pin(videos::download(LABEL, &*self.0.clock, http, url, self.0.base(), self.0.headers()))
    }
}

fn answer<T>(result: Result<T, MediaFailure>) -> MediaAnswer<T> {
    MediaAnswer { result, observed: Vec::new() }
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

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    stream: bool,
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

/// The answer's video, sent within it, so that Google keeps it for the
/// job's continuation.
#[derive(Serialize)]
struct Format {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<String>,
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
        stream: true,
        input,
        response_format: Format {
            kind: "video",
            aspect_ratio: request.aspect.filter(|_| request.source.is_none()).map(|aspect| aspect.to_string()),
            resolution: request.resolution.map(VideoResolution::lowercase),
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

/// The catalog's prices of a model's tokens, per million: its input, its
/// video and its thinking, which Google prices as text.
#[derive(Clone, Copy)]
struct Rates {
    input: Option<f64>,
    video: Option<f64>,
    text: Option<f64>,
}

impl Rates {
    fn of(catalog: &Catalog, model: &str) -> Self {
        let rates = catalog
            .model(VENDOR, model)
            .and_then(|known| catalog.price(known))
            .map(|price| price.rates.as_slice())
            .unwrap_or_default();
        let usd = |unit: PriceUnit, modality: Option<Modality>| {
            rates
                .iter()
                .find(|rate| rate.unit == unit && rate.modality == modality && rate.currency == demi_provider_common::media_catalog::Currency::Usd)
                .map(|rate| rate.amount)
        };
        Self {
            input: usd(PriceUnit::MillionInputTokens, None),
            video: usd(PriceUnit::MillionOutputTokens, Some(Modality::Video)),
            text: usd(PriceUnit::MillionOutputTokens, Some(Modality::Text)),
        }
    }

    /// What `usage` costs; none when a kind it has has no rate.
    fn cost(self, usage: &Usage) -> Option<Cost> {
        let video = usage.video_tokens();
        let text = usage.total_output_tokens.saturating_sub(video) + usage.total_thought_tokens;
        let part = |tokens: u64, rate: Option<f64>| if tokens == 0 { Some(0.0) } else { rate.map(|rate| tokens as f64 * rate / 1e6) };
        let amount = part(usage.total_input_tokens, self.input)? + part(video, self.video)? + part(text, self.text)?;
        Some(Cost { amount, currency: Currency::Usd })
    }
}

/// An event of the stream; a failure Google sends outside an event has no
/// type, only its `error`.
#[derive(Deserialize)]
struct Event {
    #[serde(default)]
    event_type: String,
    #[serde(default)]
    interaction: Option<Interaction>,
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    error: Option<Said>,
}

#[derive(Deserialize)]
struct Interaction {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    errors: Vec<Said>,
}

#[derive(Deserialize)]
struct Delta {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    uri: Option<String>,
    /// The video, as base64.
    #[serde(default)]
    data: Option<String>,
}

#[derive(Deserialize)]
struct Said {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    total_input_tokens: u64,
    #[serde(default)]
    total_output_tokens: u64,
    #[serde(default)]
    total_thought_tokens: u64,
    #[serde(default)]
    output_tokens_by_modality: Vec<ModalityTokens>,
}

#[derive(Deserialize)]
struct ModalityTokens {
    modality: String,
    tokens: u64,
}

impl Usage {
    fn video_tokens(&self) -> u64 {
        self.output_tokens_by_modality.iter().filter(|count| count.modality == "video").map(|count| count.tokens).sum()
    }

    /// The counts, thinking among the output, as the ledger keeps them.
    fn counts(&self) -> TokenUsage {
        TokenUsage {
            input_tokens: self.total_input_tokens,
            output_tokens: self.total_output_tokens + self.total_thought_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    }
}

/// What the stream has said so far.
struct Read {
    events: BoxStream<'static, Result<String, MediaFailure>>,
    rates: Rates,
    /// The video, once its step sent it.
    video: Option<DoneVideo>,
}

/// The job's steps, as its events tell them: its id, that it runs, then its
/// video or its failure; a stream that ends before either is unreadable.
fn read_events(events: BoxStream<'static, Result<String, MediaFailure>>, rates: Rates) -> BoxStream<'static, MediaAnswer<JobStep>> {
    stream::unfold(Some(Read { events, rates, video: None }), |read| async move {
        let mut read = read?;
        loop {
            let Some(event) = read.events.next().await else {
                return Some((answer(Err(unreadable("its answer ended before the video".to_owned()))), None));
            };
            let data = match event {
                Ok(data) => data,
                Err(failure) => return Some((answer(Err(failure)), None)),
            };
            if data == "[DONE]" {
                continue;
            }
            let event: Event = match serde_json::from_str(&data) {
                Ok(event) => event,
                Err(error) => {
                    let failure = unreadable(format!("its event could not be read: {error}"));
                    return Some((answer(Err(failure)), None));
                }
            };
            match read.step(event) {
                Ok(None) => {}
                Ok(Some(step)) => {
                    let ended = matches!(step, JobStep::State(JobState::Done(_) | JobState::Failed { .. }));
                    return Some((answer(Ok(step)), (!ended).then_some(read)));
                }
                Err(failure) => return Some((answer(Err(failure)), None)),
            }
        }
    })
    .boxed()
}

impl Read {
    /// The step `event` tells, if any.
    fn step(&mut self, event: Event) -> Result<Option<JobStep>, MediaFailure> {
        match event.event_type.as_str() {
            "interaction.created" => {
                let id = event.interaction.and_then(|interaction| interaction.id);
                let id = id.ok_or_else(|| unreadable("its created interaction named no id".to_owned()))?;
                Ok(Some(JobStep::Asked(id)))
            }
            "interaction.status_update" => Ok(Some(JobStep::State(JobState::Running { progress: None }))),
            "step.delta" => {
                if let Some(delta) = event.delta.filter(|delta| delta.kind.as_deref() == Some("video")) {
                    self.video = Some(video_of(delta)?);
                }
                Ok(None)
            }
            // A key's daily limit comes as the stream's error, after the
            // job was named; a refusal of its input, as a line of JSON
            // outside an event.
            "error" | "" if event.error.is_some() => match event.error {
                Some(said) if said.code.as_deref() == Some("rate_limit_exceeded") => Err(MediaFailure::UsageLimit {
                    retry_at: None,
                    trace: VendorTrace { code: said.code, id: None },
                }),
                said => Ok(Some(failed(said.into_iter().collect()))),
            },
            _ => {
                let Some(interaction) = event.interaction else {
                    return Ok(None);
                };
                match interaction.status.as_deref() {
                    Some("completed") => {
                        let video = self.video.take().ok_or_else(|| unreadable("its finished job sent no video".to_owned()))?;
                        let usage = interaction.usage.unwrap_or_default();
                        let cost = self.rates.cost(&usage);
                        Ok(Some(JobStep::State(JobState::Done(Done { video, cost, usage: Some(usage.counts()) }))))
                    }
                    Some("failed" | "cancelled" | "incomplete") => Ok(Some(failed(interaction.errors))),
                    _ => Ok(None),
                }
            }
        }
    }
}

/// The video a delta holds, within it or, as Google may still send one, at
/// an address.
fn video_of(delta: Delta) -> Result<DoneVideo, MediaFailure> {
    use base64::Engine as _;
    if let Some(data) = delta.data {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| unreadable(format!("its video could not be decoded: {error}")))?;
        return Ok(DoneVideo::Sent(within_limit(Bytes::from(bytes), "video")?));
    }
    let uri = delta.uri.ok_or_else(|| unreadable("its video came without its bytes or an address".to_owned()))?;
    let url = Url::parse(&uri).map_err(|error| unreadable(format!("its video's address could not be read: {error}")))?;
    Ok(DoneVideo::At(url))
}

/// The job's failure in Google's words.
fn failed(errors: Vec<Said>) -> JobStep {
    let said: Vec<String> = errors.into_iter().filter_map(|said| said.message).collect();
    let message = if said.is_empty() { "the job failed without a reason".to_owned() } else { said.join("; ") };
    JobStep::State(JobState::Failed { message, trace: Default::default() })
}
