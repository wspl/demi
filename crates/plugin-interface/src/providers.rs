//! The port's Providers service (`plugins.md` § The contract, `media.md`):
//! the media models of one kind of the user's provider entries with what
//! the catalog states of each and the entries that offer it, one image
//! made through an entry's account or key, and one video's job, asked for
//! and then followed to its video. The backend chooses the entry and the
//! account, rotates them, renews their sign-in, runs the job and records
//! the call in the usage ledger; no token or key reaches the plugin.

use demi_shared_types::{B64Bytes, Cost, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The models of one kind the user's entries offer (`media.md` § What the
/// agent is told): newest first by release date, those without one last.
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
    /// not, ChatGPT, is shown as asked (`images.md` § Commands).
    pub confirmed: bool,
    pub status: EntryStatus,
}

/// Why an entry would or would not make a call now (`media.md` § What the
/// agent is told).
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
    /// Providers (`media.md` § Which entry makes a call).
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
    /// account is the administrator's (`providers.md` § Scope).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Whether the vendor confirms `model` made the image; otherwise it is
    /// only the model asked for.
    pub confirmed: bool,
}

/// One video to make: from the prompt, with the frames and references
/// given; an option left out is the vendor's default (`videos.md`
/// § Commands).
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

/// The resolutions a video is asked in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum VideoResolution {
    #[serde(rename = "480p")]
    P480,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "4k")]
    K4,
}

impl VideoResolution {
    pub const ALL: [Self; 4] = [Self::P480, Self::P720, Self::P1080, Self::K4];

    /// As `--resolution` writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::P480 => "480p",
            Self::P720 => "720p",
            Self::P1080 => "1080p",
            Self::K4 => "4k",
        }
    }

    /// The shorter side of a picture of it, in pixels.
    pub fn short_side(self) -> u32 {
        match self {
            Self::P480 => 480,
            Self::P720 => 720,
            Self::P1080 => 1080,
            Self::K4 => 2160,
        }
    }
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

/// Why no image or video was made (`images.md` § Accounts and failures,
/// `videos.md` § Accounts and failures); the plugin words it for its kind.
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
    },
    /// The vendor refused the entry's API key; `manages` says whether the
    /// user manages the entry, and so can check it.
    #[error("{vendor} refused the API key")]
    KeyRefused { vendor: String, manages: bool },
    /// The vendor refused the prompt or an image, in its own words.
    #[error("{vendor} refused the request: {message}")]
    Refused { vendor: String, message: String },
    /// The vendor refused the shape asked for, naming the shapes it takes.
    #[error("{vendor} takes none of the shape asked for")]
    Shape { vendor: String, takes: Vec<String> },
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
    Unanswered { vendor: String, cause: String },
    /// The vendor failed the video's job, in its own words.
    #[error("{vendor} failed the video: {message}")]
    JobFailed { vendor: String, message: String },
    /// The job expired at the vendor before it ran.
    #[error("the job expired at {vendor} before it ran")]
    JobExpired { vendor: String },
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
