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

/// A vendor whose image models an entry makes, by its models.dev id: a
/// subscription's (ChatGPT's are OpenAI's, Grok's xAI's) or a key's. Each
/// is a row of Settings › Agent › Image Generation, in this order
/// (`images.md` § Which entry makes an image).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub enum ImageVendor {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "google")]
    Google,
    #[serde(rename = "xai")]
    Xai,
}

impl ImageVendor {
    pub const ALL: [Self; 3] = [Self::OpenAi, Self::Google, Self::Xai];

    /// Its models.dev id.
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Google => "google",
            Self::Xai => "xai",
        }
    }

    /// The vendor of models.dev id `id`, when it makes images.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|vendor| vendor.id() == id)
    }

    /// How the user knows it.
    pub fn name(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Google => "Google",
            Self::Xai => "xAI",
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
