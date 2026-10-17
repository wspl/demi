//! What the product shows about a provider entry besides its catalog: the
//! wire format it speaks, whether its credential is usable, whether it can
//! run, and its subscription accounts (`providers.md` § Provider contract, §
//! Credential vault). The backend reads the states when it answers and never
//! stores them, so they accept unknown fields.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{Nullable, Timestamp};

/// The wire format an `openai` entry speaks (`providers.md` § Endpoints): the
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
}

impl VideoShape {
    /// The shapes a compatible entry may choose.
    pub const CHOSEN: [Self; 3] = [Self::XaiVideos, Self::OpenaiVideos, Self::OpenrouterVideos];

    /// How a call's words name it, such as `Veo takes no --seed`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Grok => "Grok",
            Self::XaiVideos => "xAI Videos",
            Self::Veo => "Veo",
            Self::OpenaiVideos => "OpenAI Videos",
            Self::OpenrouterVideos => "OpenRouter Videos",
        }
    }
}

serde_plain::derive_display_from_serialize!(VideoShape);
serde_plain::derive_fromstr_from_deserialize!(VideoShape);

/// The kinds of media an entry may make, each with its shape and switch
/// (`media.md` § Which entries make which media). Audio joins as its
/// commands are built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
}

impl MediaKind {
    pub const ALL: [Self; 2] = [Self::Image, Self::Video];

    /// The word for what the kind makes, as a sentence uses it: `images`,
    /// `videos`.
    pub fn plural(self) -> &'static str {
        match self {
            Self::Image => "images",
            Self::Video => "videos",
        }
    }

    /// The name of the entry's switch for it, as Settings shows it.
    pub fn switch(self) -> &'static str {
        match self {
            Self::Image => "Image Generation",
            Self::Video => "Video Generation",
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
}

impl MediaShapes {
    /// Whether the entry makes `kind`.
    pub fn makes(&self, kind: MediaKind) -> bool {
        match kind {
            MediaKind::Image => self.image.is_some(),
            MediaKind::Video => self.video.is_some(),
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
    /// to sign in again (`claude-code.md` § Accounts and sign-in); rotation
    /// passes over it until a login replaces it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub signed_out: bool,
}

/// What a login waits for (`claude-code.md` § Accounts and sign-in): it is
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
/// waits (`providers.md` § Login and publication). When the login ends is
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
