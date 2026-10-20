//! What the product shows about a provider entry besides its catalog: the
//! wire format it speaks, whether its credential is usable, whether it can
//! run, and its subscription accounts (`crates/provider-common/src/lib.rs`,
//! `crates/backend-providers/src/vault/entries.rs`). The backend reads the
//! states when it answers and never
//! stores them, so they accept unknown fields.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{Nullable, Timestamp};

/// The wire format an `openai` entry speaks (`crates/provider-common/src/endpoint.rs`): the
/// Responses API, or Chat Completions as OpenAI-compatible vendors serve it.
/// An entry that names none speaks Responses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum WireApi {
    /// `…/responses`.
    #[default]
    #[serde(rename = "responses")]
    Responses,
    /// `…/chat/completions`.
    #[serde(rename = "chat-completions")]
    ChatCompletions,
}

serde_plain::derive_display_from_serialize!(WireApi);

/// How an entry's API makes images, its *image shape* (`media.md` § Which
/// entries make which media): a vendor entry's is its vendor's, a
/// compatible entry's the one its user chose, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum ImageShape {
    /// The ChatGPT subscription's image endpoints.
    #[serde(rename = "chatgpt")]
    Chatgpt,
    /// The Grok subscription's image endpoints.
    #[serde(rename = "grok")]
    Grok,
    /// `…/images/generations` and `…/images/edits`, as OpenAI's API and
    /// xAI's serve them.
    #[serde(rename = "openai-images")]
    OpenaiImages,
    /// Gemini's `generateContent`.
    #[serde(rename = "gemini")]
    Gemini,
    /// ByteDance's Seedream through Ark's `…/images/generations`.
    #[serde(rename = "seedream")]
    Seedream,
    /// MiniMax's `/v1/image_generation`.
    #[serde(rename = "minimax")]
    Minimax,
}

impl ImageShape {
    /// The shapes a compatible entry may choose.
    pub const CHOSEN: [Self; 1] = [Self::OpenaiImages];
}

serde_plain::derive_display_from_serialize!(ImageShape);
serde_plain::derive_fromstr_from_deserialize!(ImageShape);

/// How an entry's API makes videos, its *video shape* (`videos.md`
/// § Vendors): a vendor entry's is its vendor's, a compatible entry's the
/// one its user chose, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum VideoShape {
    /// The Grok subscription's video endpoints.
    #[serde(rename = "grok")]
    Grok,
    /// `…/videos/generations` and `…/videos/<request_id>`, as xAI's API
    /// serves them.
    #[serde(rename = "xai-videos")]
    XaiVideos,
    /// Gemini's `predictLongRunning` and its operations.
    #[serde(rename = "veo")]
    Veo,
    /// `…/videos`, `…/videos/<id>` and `…/videos/<id>/content`, as OpenAI's
    /// Videos API served them and vLLM-Omni serves them.
    #[serde(rename = "openai-videos")]
    OpenaiVideos,
    /// OpenRouter's `…/videos` jobs.
    #[serde(rename = "openrouter-videos")]
    OpenrouterVideos,
    /// Ark's `…/contents/generations/tasks`, as BytePlus and Volcengine
    /// serve Seedance.
    #[serde(rename = "ark-tasks")]
    ArkTasks,
    /// MiniMax's video generation, its H3 models' v2 tasks and Hailuo's v1
    /// ones.
    #[serde(rename = "hailuo")]
    Hailuo,
}

impl VideoShape {
    /// The shapes a compatible entry may choose.
    pub const CHOSEN: [Self; 4] = [Self::XaiVideos, Self::OpenaiVideos, Self::ArkTasks, Self::OpenrouterVideos];

    /// How a call's words name it, such as `Veo takes no --seed`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Grok => "Grok",
            Self::XaiVideos => "xAI Videos",
            Self::Veo => "Veo",
            Self::OpenaiVideos => "OpenAI Videos",
            Self::OpenrouterVideos => "OpenRouter Videos",
            Self::ArkTasks => "Ark Tasks",
            Self::Hailuo => "Hailuo",
        }
    }
}

serde_plain::derive_display_from_serialize!(VideoShape);
serde_plain::derive_fromstr_from_deserialize!(VideoShape);

/// The resolution a video is asked in, as `--resolution` writes it: a
/// height, `768p`, or a count of thousands of pixels across, `2k`
/// (`videos.md` § Commands). Each vendor's request writes it as that vendor
/// does, `768P` for MiniMax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String", extend("pattern" = r"^[0-9]+[pPkK]$", "patternDescription" = "a height such as 720p, or 2k or 4k"))]
pub enum VideoResolution {
    /// The picture's height in pixels, its shorter side for a landscape
    /// video, such as 720.
    Height(u32),
    /// Thousands of pixels across, such as 4 for `4k`.
    K(u32),
}

impl VideoResolution {
    /// As `--resolution` writes it: `768p`, `4k`.
    pub fn lowercase(self) -> String {
        match self {
            Self::Height(height) => format!("{height}p"),
            Self::K(thousands) => format!("{thousands}k"),
        }
    }

    /// As MiniMax writes it: `768P`, `2K`.
    pub fn uppercase(self) -> String {
        self.lowercase().to_uppercase()
    }

    /// The shorter side of a 16:9 picture of it, in pixels: a height's own;
    /// for `<n>k`, 540 n, as 4k is 3840×2160.
    pub fn short_side(self) -> u32 {
        match self {
            Self::Height(height) => height,
            Self::K(thousands) => thousands.saturating_mul(540),
        }
    }

    /// Whether a picture of `width`×`height` is of it: its shorter side the
    /// height asked; for `<n>k`, its longer side from 960 n to 1280 n
    /// pixels, as vendors call 1920 to 2560 across 2K and 3840 to 4096 4K.
    pub fn fits(self, width: u32, height: u32) -> bool {
        match self {
            Self::Height(asked) => width.min(height) == asked,
            Self::K(thousands) => {
                (thousands.saturating_mul(960)..=thousands.saturating_mul(1280)).contains(&width.max(height))
            }
        }
    }
}

impl TryFrom<String> for VideoResolution {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let unit = text.chars().last().map(|unit| unit.to_ascii_lowercase());
        let number = text.get(..text.len().saturating_sub(1)).and_then(|digits| digits.parse::<u32>().ok()).filter(|n| *n > 0);
        match (number, unit) {
            (Some(height), Some('p')) => Ok(Self::Height(height)),
            (Some(thousands), Some('k')) => Ok(Self::K(thousands)),
            _ => Err(format!("{text:?} is no resolution: write a height such as 720p, or 2k or 4k")),
        }
    }
}

impl From<VideoResolution> for String {
    fn from(resolution: VideoResolution) -> Self {
        resolution.lowercase()
    }
}

/// How an entry's API makes audio, its *audio shape* (`audio.md`
/// § Vendors): a vendor entry's is its vendor's, a compatible entry's the
/// one its user chose, or none. A shape makes some of speech, music and
/// sound effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum AudioShape {
    /// `…/audio/speech`, as OpenAI's API serves it and compatible servers
    /// such as vLLM-Omni do.
    #[serde(rename = "openai-speech")]
    OpenaiSpeech,
    /// Gemini's speech and Lyria's music, through the Interactions API.
    #[serde(rename = "gemini")]
    Gemini,
    /// ElevenLabs' speech, music and sound effects.
    #[serde(rename = "elevenlabs")]
    Elevenlabs,
    /// ByteDance's Seed speech and Seed Audio's sound effects, through a
    /// BytePlus Speech or Doubao Speech key.
    #[serde(rename = "seed")]
    Seed,
    /// MiniMax's speech, `/v1/t2a_v2`.
    #[serde(rename = "minimax")]
    Minimax,
}

impl AudioShape {
    /// The shapes a compatible entry may choose.
    pub const CHOSEN: [Self; 1] = [Self::OpenaiSpeech];

    /// The kinds of audio it makes.
    pub fn makes(self) -> &'static [AudioKind] {
        match self {
            Self::OpenaiSpeech => &[AudioKind::Speech],
            Self::Gemini => &[AudioKind::Speech, AudioKind::Music],
            Self::Elevenlabs => &[AudioKind::Speech, AudioKind::Music, AudioKind::Effect],
            Self::Seed => &[AudioKind::Speech, AudioKind::Effect],
            Self::Minimax => &[AudioKind::Speech],
        }
    }

    /// How a call of `kind` names it, such as `Lyria takes no --duration`.
    pub fn name(self, kind: AudioKind) -> &'static str {
        match (self, kind) {
            (Self::OpenaiSpeech, _) => "OpenAI Speech",
            (Self::Gemini, AudioKind::Music) => "Lyria",
            (Self::Gemini, _) => "Gemini",
            (Self::Elevenlabs, _) => "ElevenLabs",
            (Self::Seed, AudioKind::Speech) => "Seed speech",
            (Self::Seed, _) => "Seed Audio",
            (Self::Minimax, _) => "MiniMax speech",
        }
    }
}

serde_plain::derive_display_from_serialize!(AudioShape);
serde_plain::derive_fromstr_from_deserialize!(AudioShape);

/// What an audio model makes, and what a call of `demi audio` asks for
/// (`audio.md` § Commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AudioKind {
    Speech,
    Music,
    Effect,
}

impl AudioKind {
    pub const ALL: [Self; 3] = [Self::Speech, Self::Music, Self::Effect];

    /// As a sentence says it: `speech`, `music`, `sound effects`.
    pub fn words(self) -> &'static str {
        match self {
            Self::Speech => "speech",
            Self::Music => "music",
            Self::Effect => "sound effects",
        }
    }
}

serde_plain::derive_display_from_serialize!(AudioKind);
serde_plain::derive_fromstr_from_deserialize!(AudioKind);

/// The kinds of media an entry may make, each with its shape and switch
/// (`media.md` § Which entries make which media).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

impl MediaKind {
    pub const ALL: [Self; 3] = [Self::Image, Self::Video, Self::Audio];

    /// The word for what the kind makes, as a sentence uses it: `images`,
    /// `videos`, `audio`.
    pub fn plural(self) -> &'static str {
        match self {
            Self::Image => "images",
            Self::Video => "videos",
            Self::Audio => "audio",
        }
    }

    /// The name of the entry's switch for it, as Settings shows it.
    pub fn switch(self) -> &'static str {
        match self {
            Self::Image => "Image Generation",
            Self::Video => "Video Generation",
            Self::Audio => "Audio Generation",
        }
    }
}

serde_plain::derive_display_from_serialize!(MediaKind);
serde_plain::derive_fromstr_from_deserialize!(MediaKind);

/// The shapes in which an entry makes each kind of media, none for a kind
/// it does not make.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MediaShapes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageShape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoShape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioShape>,
}

impl MediaShapes {
    /// Whether the entry makes `kind`.
    pub fn makes(&self, kind: MediaKind) -> bool {
        match kind {
            MediaKind::Image => self.image.is_some(),
            MediaKind::Video => self.video.is_some(),
            MediaKind::Audio => self.audio.is_some(),
        }
    }
}

/// Whether a provider's credential is present and usable. Reading it never
/// makes an inference request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AuthState {
    Unknown {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        message: Option<String>,
    },
    Authenticated {
        /// The account the credential acts for, when the provider can name
        /// it.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        account_label: Option<String>,
    },
    Unauthenticated {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        message: Option<String>,
    },
    Error {
        message: String,
    },
}

/// Whether a provider can run requests at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RuntimeState {
    Unknown {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        message: Option<String>,
    },
    Ready {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        message: Option<String>,
    },
    Unavailable {
        message: String,
    },
    Error {
        message: String,
    },
}

/// A subscription account's public metadata; never token material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    /// The account's id within its entry.
    pub id: String,
    /// What the user knows the account by, such as an email address.
    pub label: String,
    /// A second line, such as the plan or the issuer.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub detail: Option<String>,
    /// When the account was last stored or refreshed.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    pub updated_at: Option<Timestamp>,
    /// The vendor refused the account's sign-in, and it waits for the user
    /// to sign in again (`crates/provider-claude-code/src/account.rs`); rotation
    /// passes over it until a login replaces it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub signed_out: bool,
}

/// What a login waits for (`crates/provider-claude-code/src/account.rs`): it is
/// `preparing` from its start, and `waiting_for_user` once it has a link or
/// a code to show. A Claude sign-in waits for the Claude Code machine, for
/// the CLI's install and for the CLI's start in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoginStage {
    Preparing,
    /// The Claude Code machine's access is acquired: its runner woken or
    /// awaited.
    WaitingForMachine,
    /// The CLI is installed on the machine.
    Installing,
    /// The CLI runs and has not printed its link yet.
    Starting,
    /// The user opens the link, enters the code or pastes one back.
    WaitingForUser,
}

impl LoginStage {
    /// The stage's name, as the API and the server log write it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::WaitingForMachine => "waiting_for_machine",
            Self::Installing => "installing",
            Self::Starting => "starting",
            Self::WaitingForUser => "waiting_for_user",
        }
    }
}

/// What a login asks the user to do, reported while the login
/// waits (`crates/backend-providers/src/vault/logins.rs`). When the login ends is
/// the vault's, not the vendor's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginPending {
    /// The address the user opens to confirm the login.
    pub verification_url: String,
    /// The one-time code the user enters there; null when the address
    /// carries it.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub user_code: Option<String>,
    /// The code the vendor's sign-in refused, while it waits for another;
    /// null otherwise.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<RefusedCode>")]
    pub code_error: Option<RefusedCode>,
}

/// A code the user pasted that the vendor's sign-in refused: the `code`th
/// the login took, counting from one, and why, in the sign-in's own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RefusedCode {
    pub code: u32,
    pub message: String,
}
