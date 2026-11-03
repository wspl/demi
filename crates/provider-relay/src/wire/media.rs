//! The media wire: what a borrower's maker of images, videos or audio asks
//! the lender over HTTP, under [`MEDIA_ROUTE`], and what it answers.
//!
//! ```text
//! GET    makers/{kind}        the maker, as [`LentMaker`] says it
//! POST   image                one image                 -> [`WireMadeImage`]
//! POST   audio                one file of audio         -> [`WireMadeAudio`]
//! POST   audio/voices         a speech model's voices   -> [`WireVoice`]s
//! POST   video                a video's ask             -> [`WireAskedVideo`]
//! GET    video/{job}          the job's state           -> [`WireJobState`]
//! GET    video/{job}/file     the finished job's video, its bytes
//! DELETE video/{job}          cancels the job
//! ```
//!
//! A call the lender's vendor did not answer as asked is answered HTTP 422
//! with a [`WireMediaFailure`], its kind and code alone. The lender's
//! account, the vendor's words and its trace stay with the lender, as a
//! turn's do (`crate::wire`).

use demi_provider_common::ErrorCode;
use demi_provider_common::audio::{AudioFormatAsk, AudioOption, AudioRequest, Voice};
use demi_provider_common::images::{Aspect, ImageRequest, InputImage};
use demi_provider_common::media::MediaFailure;
use demi_provider_common::videos::{JobState, VideoOption, VideoRequest};
use demi_shared_types::{AudioKind, B64Bytes, Cost, TokenUsage, VideoResolution};
use serde::{Deserialize, Serialize};

use super::WireBytes;

/// The routes of a borrowed entry's media.
pub const MEDIA_ROUTE: &str = "/api/relay/media";

/// What the lent entry's maker of one kind says of itself, which the
/// borrower's maker answers its own questions with between calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LentMaker {
    /// How the user knows who answers, such as `OpenAI`.
    pub vendor_name: String,
    /// The vendor's id in the media catalog; none for a compatible entry.
    pub vendor: Option<String>,
    pub confirms_model: bool,
    pub needs_voice: bool,
    /// The slowest and fastest factors of the voice's speed it takes.
    pub speeds: Option<[f64; 2]>,
    pub models: Vec<LentMediaModel>,
}

/// A model the lent maker offers, with what a request of it takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LentMediaModel {
    pub id: String,
    pub name: Option<String>,
    pub released: Option<String>,
    pub description: Option<String>,
    /// For an audio model, the kinds it makes.
    pub kinds: Vec<AudioKind>,
    /// What a request of it takes: one for a video model, one for each
    /// kind an audio model makes, none for an image model.
    pub takes: Vec<Takes>,
}

/// What a request of one model, of one kind for audio, takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Takes {
    /// The audio kind; none for a video.
    pub kind: Option<AudioKind>,
    /// How a call's words name the request it takes, such as `Veo`.
    pub shape_name: String,
    /// The options its request has no field for.
    pub lacks: Vec<WireOption>,
}

/// An option of `demi video generate` or `demi audio`, as a request lacks
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireOption {
    Image,
    LastFrame,
    Reference,
    Duration,
    Aspect,
    Resolution,
    Audio,
    Seed,
    Instructions,
    Speed,
    Instrumental,
    Loop,
}

impl From<VideoOption> for WireOption {
    fn from(option: VideoOption) -> Self {
        match option {
            VideoOption::Image => Self::Image,
            VideoOption::LastFrame => Self::LastFrame,
            VideoOption::Reference => Self::Reference,
            VideoOption::Duration => Self::Duration,
            VideoOption::Aspect => Self::Aspect,
            VideoOption::Resolution => Self::Resolution,
            VideoOption::Audio => Self::Audio,
            VideoOption::Seed => Self::Seed,
        }
    }
}

impl From<AudioOption> for WireOption {
    fn from(option: AudioOption) -> Self {
        match option {
            AudioOption::Instructions => Self::Instructions,
            AudioOption::Speed => Self::Speed,
            AudioOption::Duration => Self::Duration,
            AudioOption::Instrumental => Self::Instrumental,
            AudioOption::Seed => Self::Seed,
            AudioOption::Loop => Self::Loop,
        }
    }
}

impl WireOption {
    /// The option of a video's request; none for one only audio has.
    pub fn video(self) -> Option<VideoOption> {
        Some(match self {
            Self::Image => VideoOption::Image,
            Self::LastFrame => VideoOption::LastFrame,
            Self::Reference => VideoOption::Reference,
            Self::Duration => VideoOption::Duration,
            Self::Aspect => VideoOption::Aspect,
            Self::Resolution => VideoOption::Resolution,
            Self::Audio => VideoOption::Audio,
            Self::Seed => VideoOption::Seed,
            Self::Instructions | Self::Speed | Self::Instrumental | Self::Loop => return None,
        })
    }

    /// The option of an audio request; none for one only a video has.
    pub fn audio(self) -> Option<AudioOption> {
        Some(match self {
            Self::Instructions => AudioOption::Instructions,
            Self::Speed => AudioOption::Speed,
            Self::Duration => AudioOption::Duration,
            Self::Instrumental => AudioOption::Instrumental,
            Self::Seed => AudioOption::Seed,
            Self::Loop => AudioOption::Loop,
            Self::Image | Self::LastFrame | Self::Reference | Self::Aspect | Self::Resolution | Self::Audio => {
                return None;
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireAspect {
    pub width: u32,
    pub height: u32,
}

impl From<Aspect> for WireAspect {
    fn from(aspect: Aspect) -> Self {
        Self { width: aspect.width, height: aspect.height }
    }
}

impl From<WireAspect> for Aspect {
    fn from(aspect: WireAspect) -> Self {
        Self { width: aspect.width, height: aspect.height }
    }
}

impl From<&InputImage> for WireBytes {
    fn from(image: &InputImage) -> Self {
        Self { data: image.bytes.clone().into(), media_type: image.media_type.to_owned() }
    }
}

/// The image `bytes` hold, when its type is one an edit sends; a request
/// with another is no borrower's and is refused.
pub fn input_image(bytes: WireBytes) -> Option<InputImage> {
    let media_type = ["image/png", "image/jpeg", "image/webp"].into_iter().find(|known| *known == bytes.media_type)?;
    Some(InputImage { media_type, bytes: bytes.data.into_bytes() })
}

/// `POST image`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireImageRequest {
    pub model: String,
    pub prompt: String,
    pub aspect: Option<WireAspect>,
    pub transparent: bool,
    pub images: Vec<WireBytes>,
}

impl From<&ImageRequest> for WireImageRequest {
    fn from(request: &ImageRequest) -> Self {
        Self {
            model: request.model.clone(),
            prompt: request.prompt.clone(),
            aspect: request.aspect.map(Into::into),
            transparent: request.transparent,
            images: request.images.iter().map(Into::into).collect(),
        }
    }
}

impl WireImageRequest {
    /// The request as the lender's maker takes it; none when an image is
    /// of a type no edit sends.
    pub fn request(self) -> Option<ImageRequest> {
        Some(ImageRequest {
            model: self.model,
            prompt: self.prompt,
            aspect: self.aspect.map(Into::into),
            transparent: self.transparent,
            images: self.images.into_iter().map(input_image).collect::<Option<_>>()?,
        })
    }
}

/// The answer of `POST image`: the image and the counts the vendor
/// reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireMadeImage {
    pub data: B64Bytes,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireAudioFormat {
    Mp3,
    Wav,
    Opus,
    Flac,
}

impl From<AudioFormatAsk> for WireAudioFormat {
    fn from(format: AudioFormatAsk) -> Self {
        match format {
            AudioFormatAsk::Mp3 => Self::Mp3,
            AudioFormatAsk::Wav => Self::Wav,
            AudioFormatAsk::Opus => Self::Opus,
            AudioFormatAsk::Flac => Self::Flac,
        }
    }
}

impl From<WireAudioFormat> for AudioFormatAsk {
    fn from(format: WireAudioFormat) -> Self {
        match format {
            WireAudioFormat::Mp3 => Self::Mp3,
            WireAudioFormat::Wav => Self::Wav,
            WireAudioFormat::Opus => Self::Opus,
            WireAudioFormat::Flac => Self::Flac,
        }
    }
}

/// `POST audio`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireAudioRequest {
    pub kind: AudioKind,
    pub model: String,
    pub text: String,
    pub voice: Option<String>,
    pub instructions: Option<String>,
    pub speed: Option<f64>,
    pub duration: Option<f64>,
    pub instrumental: bool,
    pub seed: Option<u64>,
    pub looped: bool,
    pub format: WireAudioFormat,
}

impl From<&AudioRequest> for WireAudioRequest {
    fn from(request: &AudioRequest) -> Self {
        Self {
            kind: request.kind,
            model: request.model.clone(),
            text: request.text.clone(),
            voice: request.voice.clone(),
            instructions: request.instructions.clone(),
            speed: request.speed,
            duration: request.duration,
            instrumental: request.instrumental,
            seed: request.seed,
            looped: request.looped,
            format: request.format.into(),
        }
    }
}

impl From<WireAudioRequest> for AudioRequest {
    fn from(request: WireAudioRequest) -> Self {
        Self {
            kind: request.kind,
            model: request.model,
            text: request.text,
            voice: request.voice,
            instructions: request.instructions,
            speed: request.speed,
            duration: request.duration,
            instrumental: request.instrumental,
            seed: request.seed,
            looped: request.looped,
            format: request.format.into(),
        }
    }
}

/// The shape of raw PCM a vendor sent instead of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WirePcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
}

/// The answer of `POST audio`: the audio as the vendor sent it, and what it
/// reported of the call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireMadeAudio {
    pub data: B64Bytes,
    pub pcm: Option<WirePcm>,
    pub cost: Option<Cost>,
    pub usage: Option<TokenUsage>,
}

/// `POST audio/voices`; its answer is the voices, or null for a vendor
/// that lists none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireVoicesRequest {
    pub model: String,
    pub search: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireVoice {
    pub id: String,
    pub name: Option<String>,
    pub languages: Vec<String>,
    pub description: Option<String>,
    pub own: Option<bool>,
}

impl From<Voice> for WireVoice {
    fn from(voice: Voice) -> Self {
        Self { id: voice.id, name: voice.name, languages: voice.languages, description: voice.description, own: voice.own }
    }
}

impl From<WireVoice> for Voice {
    fn from(voice: WireVoice) -> Self {
        Self { id: voice.id, name: voice.name, languages: voice.languages, description: voice.description, own: voice.own }
    }
}

/// `POST video`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireVideoRequest {
    pub model: String,
    pub prompt: String,
    pub image: Option<WireBytes>,
    pub last_frame: Option<WireBytes>,
    pub references: Vec<WireBytes>,
    pub duration: Option<u32>,
    pub aspect: Option<WireAspect>,
    pub resolution: Option<VideoResolution>,
    pub audio: Option<bool>,
    pub seed: Option<u64>,
}

impl From<&VideoRequest> for WireVideoRequest {
    fn from(request: &VideoRequest) -> Self {
        Self {
            model: request.model.clone(),
            prompt: request.prompt.clone(),
            image: request.image.as_ref().map(Into::into),
            last_frame: request.last_frame.as_ref().map(Into::into),
            references: request.references.iter().map(Into::into).collect(),
            duration: request.duration,
            aspect: request.aspect.map(Into::into),
            resolution: request.resolution,
            audio: request.audio,
            seed: request.seed,
        }
    }
}

impl WireVideoRequest {
    /// The request as the lender's maker takes it; none when an image is
    /// of a type no video's request sends.
    pub fn request(self) -> Option<VideoRequest> {
        let image = |bytes: Option<WireBytes>| bytes.map(input_image).map_or(Some(None), |image| image.map(Some));
        Some(VideoRequest {
            model: self.model,
            prompt: self.prompt,
            image: image(self.image)?,
            last_frame: image(self.last_frame)?,
            references: self.references.into_iter().map(input_image).collect::<Option<_>>()?,
            duration: self.duration,
            aspect: self.aspect.map(Into::into),
            resolution: self.resolution,
            audio: self.audio,
            seed: self.seed,
        })
    }
}

/// The answer of `POST video`: the job's id at the lender, which is not
/// the vendor's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireAskedVideo {
    pub job: String,
}

/// The answer of `GET video/{job}`. A finished job's video is at
/// `video/{job}/file`; a failed one's words are the lender's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum WireJobState {
    Queued,
    Running { progress: Option<u8> },
    Done { cost: Option<Cost>, usage: Option<TokenUsage> },
    Failed,
    Expired,
}

impl From<&JobState> for WireJobState {
    fn from(state: &JobState) -> Self {
        match state {
            JobState::Queued => Self::Queued,
            JobState::Running { progress } => Self::Running { progress: *progress },
            JobState::Done(done) => Self::Done { cost: done.cost, usage: done.usage },
            JobState::Failed { .. } => Self::Failed,
            JobState::Expired => Self::Expired,
        }
    }
}

/// Why a media call was not answered as asked, with HTTP 422: the kind of
/// the vendor's failure and its code ([`ErrorCode::as_str`]), with a refused
/// shape's shapes it takes, or the lender's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum WireMediaFailure {
    /// Every account the lender could send it on is used up.
    UsageLimit,
    /// The lender's account or key could not be used.
    Unusable,
    Refused { code: Option<String> },
    /// The vendor takes none of the shape asked for, but those it names.
    Shape { takes: Vec<String>, code: Option<String> },
    Unanswered { code: Option<String> },
    /// The lent entry makes no media of the kind now: its switch is off, or
    /// a request named an image of a type no request sends.
    Unmade,
    /// The job is no job of the key's at the lender: one it never asked
    /// for, or one the lender no longer keeps.
    NoJob,
}

impl From<&MediaFailure> for WireMediaFailure {
    fn from(failure: &MediaFailure) -> Self {
        let code = |code: &Option<ErrorCode>| code.as_ref().map(|code| code.as_str().to_owned());
        match failure {
            MediaFailure::UsageLimit { .. } => Self::UsageLimit,
            MediaFailure::SignIn(_) | MediaFailure::KeyRefused { .. } => Self::Unusable,
            MediaFailure::Refused { code: own, .. } => Self::Refused { code: code(own) },
            MediaFailure::Shape { takes, code: own, .. } => Self::Shape { takes: takes.clone(), code: code(own) },
            MediaFailure::Unanswered { code: own, .. } => Self::Unanswered { code: code(own) },
        }
    }
}
