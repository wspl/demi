//! Models, the selection a conversation infers with, and token usage
//! (`models.md` § Request parameters).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{MAX_SAFE_INTEGER, Nullable};

/// A file type a model can read natively (`models.md` § Accepted attachment
/// types). Extensions omit the dot; `jpg` and `jpeg` are one format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FileExtension {
    Png,
    Jpg,
    Jpeg,
    Gif,
    Webp,
    Pdf,
    Mp4,
    Mov,
    Webm,
    M4v,
}

serde_plain::derive_display_from_serialize!(FileExtension);
serde_plain::derive_fromstr_from_deserialize!(FileExtension);

/// The image and document types a model known to read attachments accepts
/// (`models.md` § Accepted attachment types).
pub const ATTACHMENT_FILE_EXTENSIONS: [FileExtension; 6] = [
    FileExtension::Png,
    FileExtension::Jpg,
    FileExtension::Jpeg,
    FileExtension::Gif,
    FileExtension::Webp,
    FileExtension::Pdf,
];

/// The video types, which only a model known to read video accepts.
pub const VIDEO_FILE_EXTENSIONS: [FileExtension; 4] = [
    FileExtension::Mp4,
    FileExtension::Mov,
    FileExtension::Webm,
    FileExtension::M4v,
];

/// Whether a model whose catalog lists `accepted` reads `extension`
/// natively: yes, no, or unknown (`None`) when the catalog does not say.
/// Where Demi decides whether to give a file natively, only a yes counts.
pub fn file_extension_support(
    accepted: Option<&[FileExtension]>,
    extension: FileExtension,
) -> Option<bool> {
    let accepted = accepted?;
    let alias = match extension {
        FileExtension::Jpg => Some(FileExtension::Jpeg),
        FileExtension::Jpeg => Some(FileExtension::Jpg),
        _ => None,
    };
    let listed = accepted.contains(&extension)
        || alias.is_some_and(|alias| accepted.contains(&alias));
    Some(listed)
}

/// Whether `model` is known to read at least one video type.
pub fn model_accepts_video(model: &Model) -> bool {
    VIDEO_FILE_EXTENSIONS.into_iter().any(|extension| {
        file_extension_support(model.accepted_extensions.as_deref(), extension) == Some(true)
    })
}

/// A model with its catalog facts, as a selection records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Model {
    #[garde(length(utf16, min = 1))]
    pub id: String,
    #[garde(skip)]
    pub name: String,
    /// Tokens; zero when the catalog does not know it.
    #[garde(skip)]
    pub context_window: u32,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    #[garde(skip)]
    pub input_limit: Option<u32>,
    /// The most tokens one request may generate: a positive whole number, or
    /// null when no model-specific limit is known.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    #[garde(range(min = 1))]
    pub output_limit: Option<u32>,
    #[garde(dive)]
    pub thinking: Vec<ThinkingCapability>,
    /// The types the model reads natively: `[]` for none, null when unknown.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Vec<FileExtension>>")]
    #[garde(skip)]
    pub accepted_extensions: Option<Vec<FileExtension>>,
}

/// The model a conversation infers with and how: the provider entry, the
/// model with its facts, the thinking setting and the service tier. Every
/// block records the selection that was current when it was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSelection {
    #[garde(length(utf16, min = 1))]
    pub provider_id: String,
    #[garde(dive)]
    pub model: Model,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ThinkingConfig>")]
    #[garde(dive)]
    pub thinking: Option<ThinkingConfig>,
    /// The service tier, such as the one the catalog marks Fast; null for the
    /// vendor's default.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(length(utf16, min = 1))]
    pub service_tier_id: Option<String>,
}

/// Whether a reasoning summary is asked for, and how detailed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingSummary {
    Auto,
    Concise,
    Detailed,
    Off,
    On,
}

serde_plain::derive_display_from_serialize!(ThinkingSummary);
serde_plain::derive_fromstr_from_deserialize!(ThinkingSummary);

/// One way a model can think, as its catalog offers it. Effort levels are the
/// vendor's words, such as `low` or `xhigh`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ThinkingCapability {
    Adaptive {
        #[garde(skip)]
        efforts: Vec<String>,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<String>")]
        #[garde(skip)]
        default_effort: Option<String>,
    },
    Budget {
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<u32>")]
        #[garde(skip)]
        min_budget_tokens: Option<u32>,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<u32>")]
        #[garde(skip)]
        max_budget_tokens: Option<u32>,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<u32>")]
        #[garde(skip)]
        default_budget_tokens: Option<u32>,
    },
    Effort {
        #[garde(skip)]
        efforts: Vec<String>,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<String>")]
        #[garde(skip)]
        default_effort: Option<String>,
        #[garde(skip)]
        summaries: Vec<ThinkingSummary>,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<ThinkingSummary>")]
        #[garde(skip)]
        default_summary: Option<ThinkingSummary>,
    },
    /// Thinking can be turned off.
    Disabled {},
}

/// The thinking setting a selection makes, which each provider maps onto its
/// vendor's option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ThinkingConfig {
    Adaptive {
        #[garde(skip)]
        effort: String,
    },
    Budget {
        #[garde(skip)]
        budget_tokens: u32,
    },
    Effort {
        #[garde(skip)]
        effort: String,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<ThinkingSummary>")]
        #[garde(skip)]
        summary: Option<ThinkingSummary>,
    },
    Disabled {},
}

/// The tokens one completed request used, as the provider reported them.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenUsage {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub input_tokens: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub output_tokens: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub cache_read_tokens: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub cache_write_tokens: u64,
}

