//! The port's Providers service:
//! the media models of one kind of the user's provider entries with what
//! the catalog states of each and the entries that offer it, one image
//! made through an entry's account or key, one video's job, asked for and
//! then followed to its video, and one audio file with the voices a speech
//! model speaks in. The backend chooses the entry and the
//! account, rotates them, renews their sign-in, runs the job and records
//! the call in the usage ledger; no token or key reaches the plugin.

pub use demi_shared_types::VideoResolution;
use demi_shared_types::{AudioKind, B64Bytes, Cost, Timestamp, VendorTrace};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The models of one kind the user's entries offer: newest first by
/// release date, those without one last.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaListing {
    pub models: Vec<MediaModel>,
    /// The entries whose models could not be listed, each with why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unlisted: Vec<MediaUnlisted>,
    /// Why no entry offers any model, when none does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub none: Option<MediaRefusal>,
}

/// A model, with every fact the catalog of media models states of it and
/// the entries that offer it in the order a call tries them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaModel {
    /// The ID `--model` takes.
    pub id: String,
    /// The vendor's name, such as `OpenAI`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// The vendor's id in the catalog, such as `openai`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_id: Option<String>,
    /// The vendor's name for the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The vendor's one-line description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The release date, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<String>,
    /// For an alias, the model it names now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_of: Option<String>,
    /// The aliases that name it now.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// What an image model takes, as the catalog states it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageFacts>,
    /// What a video model takes, as the catalog states it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoFacts>,
    /// What an audio model makes and takes, as the catalog states it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioFacts>,
    /// What it costs through a key, as the catalog states it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<MediaPrice>,
    /// The entries that offer it, in the order a call tries them.
    pub entries: Vec<ModelEntry>,
}

/// What an image model takes; a fact the vendor does not state is absent,
/// which means unknown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageFacts {
    /// The only sizes it takes, as `1024x1536`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sizes: Option<Vec<String>>,
    /// The rule of the other sizes it takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_size: Option<CustomSizeRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratios: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolutions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_images: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparent: Option<bool>,
}

/// What a video model takes; a fact the vendor does not state is absent,
/// which means unknown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoFacts {
    /// The only durations it takes, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durations: Option<Vec<u32>>,
    /// The fewest seconds it takes, when it takes any in a range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_duration: Option<f64>,
    /// The most seconds it takes, when it takes any in a range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratios: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolutions: Option<Vec<String>>,
    /// Whether it takes the first frame, and the last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_frame: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_frame: Option<bool>,
    /// How many reference images it takes; 0 for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_reference_images: Option<u32>,
    /// Whether it makes sound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
}

/// What an audio model makes and takes; a fact the vendor does not state
/// is absent, which means unknown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioFacts {
    /// What it makes: speech, music or sound effects, as the catalog or its
    /// entry says; empty when nothing says.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<AudioKind>,
    /// For speech, its built-in voices by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voices: Option<Vec<String>>,
    /// For speech, whether the vendor lists its voices, which `demi audio
    /// voices` reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voices_listed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub languages: Option<Vec<String>>,
    /// For speech, whether it takes instructions on the delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<bool>,
    /// For speech, the longest text one call takes, in characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_characters: Option<u32>,
    /// For music and effects, the only durations it makes, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durations: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<f64>,
    /// For music, whether it takes lyrics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyrics: Option<bool>,
    /// For music, whether it makes an instrumental on request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrumental: Option<bool>,
    /// For effects, whether it makes a sound that loops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loops: Option<bool>,
    /// The formats it returns, such as `mp3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formats: Option<Vec<String>>,
}

/// The sizes a model takes besides any fixed ones.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomSizeRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiple_of: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_edge: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ratio: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_pixels: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pixels: Option<u64>,
}

/// A model's prices through a key, and the vendor's pricing page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaPrice {
    pub rates: Vec<PriceRate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

/// What one unit costs, in the vendor's currency, by the size, quality,
/// resolution or duration it depends on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PriceRate {
    /// Such as `image`, `second` or `millionInputTokens`.
    pub unit: String,
    /// `image` or `text`, for tokens the vendor prices by their kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modality: Option<String>,
    pub amount: f64,
    /// `USD` or `CNY`.
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    /// Seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}

/// An entry that offers a model, and whether a call would take it now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelEntry {
    /// The entry's name, as Settings › Providers shows it.
    pub provider: String,
    pub provider_id: String,
    /// Whether it is a subscription, whose media come with its plan, or a
    /// key, which pays for each.
    pub subscription: bool,
    /// The account's label for a user who manages the entry; none for a
    /// key or a shared instance's user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Whether its vendor confirms the model that answered; one that does
    /// not, ChatGPT, is shown as asked.
    pub confirmed: bool,
    pub status: EntryStatus,
}

/// Why an entry would or would not make a call now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum EntryStatus {
    /// The entry a call takes now.
    Now,
    /// One a call takes when those before it cannot.
    Next,
    /// Every account of it is used up, until the moment when known.
    UsedUp {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<Timestamp>,
    },
    /// Its switch for the kind is off.
    Off,
    /// A subscription with no account signed in.
    NotSignedIn,
    /// The user turned the entry off in Settings › Providers.
    TurnedOff,
}

/// An entry whose models could not be listed, and why, as a call's
/// refusal says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaUnlisted {
    pub provider: String,
    pub refusal: MediaRefusal,
}

/// One image to make: from the prompt alone, or from the prompt and the
/// images given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageAsk {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageInput>,
    /// The shape asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<ImageAspect>,
    pub transparent: bool,
    /// An image model `demi image models` lists.
    pub model: String,
    /// The entry to make it through, by its name or id, when the agent
    /// names one; else the first that can, by the order of Settings ›
    /// Providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// A shape, such as 16:9: two positive whole numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageAspect {
    pub width: u32,
    pub height: u32,
}

/// An image an edit sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageInput {
    pub kind: ImageKind,
    pub bytes: B64Bytes,
}

/// The kinds of image an edit sends, by their media types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageKind {
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/webp")]
    Webp,
}

impl ImageKind {
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }
}

/// The image made, with the entry, account and model that made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MadeImage {
    /// As the vendor answered it; the plugin reads what it is.
    pub bytes: B64Bytes,
    pub model: String,
    pub provider: String,
    pub vendor: String,
    /// The account's label, such as its email, for a user who manages the
    /// entry; none for a user of a shared instance's entries, whose
    /// account is the administrator's (`crates/backend-providers/src/vault/entries.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Whether the vendor confirms `model` made the image; otherwise it is
    /// only the model asked for.
    pub confirmed: bool,
}

/// One video to make: from the prompt, with the frames and references
/// given; an option left out is the vendor's default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoAsk {
    pub prompt: String,
    /// The first frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageInput>,
    /// The last frame, with a first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_frame: Option<ImageInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ImageInput>,
    /// Seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<ImageAspect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<VideoResolution>,
    /// Whether the model makes sound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// A video model `demi video models` lists.
    pub model: String,
    /// The entry to make it through, as [`ImageAsk::provider`] names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// An option of `demi video generate`, as a shape that has no field for it
/// is said not to take it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VideoOption {
    Image,
    LastFrame,
    Reference,
    Duration,
    Aspect,
    Resolution,
    Audio,
    Seed,
}

/// The job the vendor took: who makes it, its id at the vendor, and the
/// options the entry's shape has no field for, which were not sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AskedVideo {
    /// The vendor's id of the job.
    pub job: String,
    pub model: String,
    pub provider: String,
    pub vendor: String,
    /// The account, as [`MadeImage::account`] names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The shape's name, such as `Veo`, which a warning of an option it
    /// has no field for names.
    pub shape: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsent: Vec<VideoOption>,
}

/// What one wait for the job answers: its next state, or its video.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum VideoStep {
    /// Waiting for the vendor to start it.
    Queued,
    /// Being made, with how far along when the vendor says.
    Running {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        progress: Option<u8>,
    },
    /// The video, downloaded.
    Made { video: MadeVideo },
}

/// The video a job made, as the vendor sent it, with what it cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MadeVideo {
    /// As the vendor sent it; the plugin reads what it is.
    pub bytes: B64Bytes,
    /// The cost the vendor reported, in its documented unit; none
    /// otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
}

/// One audio file to make: speech from the text, or music or a sound
/// effect from the prompt; an option left out is the vendor's default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioAsk {
    pub kind: AudioKind,
    /// The text to read aloud, or the prompt.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    /// Seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// A sound that loops.
    #[serde(default)]
    pub looped: bool,
    pub format: AudioFileFormat,
    /// An audio model `demi audio models` lists.
    pub model: String,
    /// The entry to make it through, as [`ImageAsk::provider`] names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// The formats `--format` asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AudioFileFormat {
    #[default]
    Mp3,
    Wav,
    Opus,
    Flac,
}

impl AudioFileFormat {
    /// As a call's line names it, such as `MP3`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Mp3 => "MP3",
            Self::Wav => "WAV",
            Self::Opus => "Opus",
            Self::Flac => "FLAC",
        }
    }
}

/// An option of `demi audio`, as a shape that has no field for it is said
/// not to take it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioOption {
    Instructions,
    Speed,
    Duration,
    Instrumental,
    Seed,
    Loop,
}

/// Raw PCM's shape: signed little-endian samples, interleaved by channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PcmShape {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
}

/// The audio made, as the vendor sent it, with who made it, the options
/// its shape had no field for and what it cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MadeAudio {
    /// As the vendor sent it; the plugin reads what it is.
    pub bytes: B64Bytes,
    /// Raw PCM's shape, for a vendor that sent no file, which the plugin
    /// wraps as WAV; none for a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pcm: Option<PcmShape>,
    pub model: String,
    pub provider: String,
    pub vendor: String,
    /// The account, as [`MadeImage::account`] names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The shape's name for the call's kind, such as `Lyria`, which a
    /// warning of an option it has no field for names.
    pub shape: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsent: Vec<AudioOption>,
    /// The speed sent, the nearer end of the vendor's range, when the one
    /// asked was outside it; none when it was sent as asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    /// The cost the vendor reported, in its documented unit; none
    /// otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
}

/// The voices of a speech model, as the entry a call of it takes lists
/// them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioVoices {
    pub model: String,
    pub provider: String,
    pub vendor: String,
    /// None for an entry that lists none, whose `--voice` is sent as given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voices: Option<Vec<VoiceInfo>>,
}

/// A voice, as `demi audio voices` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceInfo {
    /// What `--voice` takes.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether it is the account's own, such as a cloned voice, rather
    /// than the vendor's; none when the vendor does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own: Option<bool>,
}

/// Why a named entry cannot make a model now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum Unable {
    /// It does not offer the model, or makes none of its kind.
    NotOffered,
    UsedUp {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<Timestamp>,
    },
    Off,
    NotSignedIn,
    TurnedOff,
}

/// Why no image or video was made; the plugin words it for its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MediaRefusal {
    /// No entry the user has on makes the kind.
    #[error("no provider makes media of the kind")]
    NoProvider,
    /// No entry makes the model asked for.
    #[error("no provider makes \"{model}\"")]
    UnknownModel { model: String },
    /// Every entry that could make the kind has its switch for it off; it
    /// names them, as Settings › Providers does.
    #[error("generation is off for {}", providers.join(", "))]
    AllOff { providers: Vec<String> },
    /// The model asked for is made by an entry whose switch for the kind is
    /// off, which it names.
    #[error("\"{model}\" is {provider}'s, whose generation is off")]
    Off { model: String, provider: String },
    /// Every entry that could make the model is used up, the first named
    /// with the moment it can be used again, when known.
    #[error("{vendor}'s usage limit is reached")]
    UsedUp {
        vendor: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resets_at: Option<Timestamp>,
        #[serde(default, skip_serializing_if = "VendorTrace::is_empty")]
        trace: VendorTrace,
    },
    /// The vendor refused the entry's API key; `manages` says whether the
    /// user manages the entry, and so can check it.
    #[error("{vendor} refused the API key")]
    KeyRefused {
        vendor: String,
        manages: bool,
        #[serde(default, skip_serializing_if = "VendorTrace::is_empty")]
        trace: VendorTrace,
    },
    /// The vendor refused the prompt or an image, in its own words.
    #[error("{vendor} refused the request: {message}")]
    Refused {
        vendor: String,
        message: String,
        #[serde(default, skip_serializing_if = "VendorTrace::is_empty")]
        trace: VendorTrace,
    },
    /// The vendor refused the shape asked for, naming the shapes it takes.
    #[error("{vendor} takes none of the shape asked for")]
    Shape {
        vendor: String,
        takes: Vec<String>,
        #[serde(default, skip_serializing_if = "VendorTrace::is_empty")]
        trace: VendorTrace,
    },
    /// The account's sign-in expired and cannot be renewed; the account is
    /// named as [`MadeImage`] names it.
    #[error("the {vendor} sign-in has expired")]
    Expired {
        vendor: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<String>,
    },
    /// The vendor failed, or did not answer in time.
    #[error("{vendor} did not answer: {cause}")]
    Unanswered {
        vendor: String,
        cause: String,
        #[serde(default, skip_serializing_if = "VendorTrace::is_empty")]
        trace: VendorTrace,
    },
    /// The vendor failed the video's job, in its own words.
    #[error("{vendor} failed the video: {message}")]
    JobFailed {
        vendor: String,
        message: String,
        #[serde(default, skip_serializing_if = "VendorTrace::is_empty")]
        trace: VendorTrace,
    },
    /// The job expired at the vendor before it ran.
    #[error("the job expired at {vendor} before it ran")]
    JobExpired { vendor: String },
    /// A speech call named no voice and the vendor has no default one.
    #[error("{vendor} needs a voice")]
    NeedsVoice { vendor: String, model: String },
    /// The model makes no audio of the kind asked for; `makes` are the
    /// kinds it does.
    #[error("\"{model}\" makes no {asked}")]
    Kind { model: String, asked: AudioKind, makes: Vec<AudioKind> },
    /// The job had not finished when the call's bound, `seconds` from the
    /// ask, passed; it still runs at the vendor under its id `job`.
    #[error("the job {job} is still running at {vendor}")]
    StillRunning { vendor: String, job: String, seconds: u64 },
    /// The entry `--provider` names cannot make the model now, saying why,
    /// with the entries that can, in the order a call tries them.
    #[error("{provider} cannot make \"{model}\"")]
    Cannot {
        provider: String,
        model: String,
        why: Unable,
        can: Vec<String>,
    },
    /// The backend could not make the call, such as a store that failed.
    #[error("{message}")]
    Failed { message: String },
}
