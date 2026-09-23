//! A provider entry's model catalog (`models.md` § Catalog sources): the
//! models the entry offers and what each can do. A capability the source does
//! not state is null, which means unknown. The catalog carries portable facts
//! only, never the name of the source it came from.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{FileExtension, Nullable, Timestamp};

/// An entry's catalog as one read of its source returned it. The backend
/// keeps it in its catalog cache and reads it back from storage, so it
/// refuses unknown fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderModelList {
    #[garde(dive)]
    pub models: Vec<ProviderModel>,
    /// The model a new selection starts with; null when the source names
    /// none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub default_model_id: Option<String>,
    /// What went wrong while the catalog was read, such as a refresh that
    /// failed and left an older copy.
    #[garde(skip)]
    pub warnings: Vec<String>,
    /// When the source was last downloaded; the Unix epoch for a list that
    /// was never fetched, such as one built into a provider.
    #[garde(skip)]
    pub source_fetched_at: Timestamp,
    /// Whether this is a copy kept after a failed refresh.
    #[garde(skip)]
    pub stale: bool,
}

/// One model of a catalog, as its source describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderModel {
    #[garde(length(utf16, min = 1))]
    pub id: String,
    #[garde(skip)]
    pub display_name: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub description: Option<String>,
    /// Tokens.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    #[garde(skip)]
    pub context_window: Option<u32>,
    /// The most tokens one request may generate; null when no
    /// model-specific limit is known (`models.md` § Output limit).
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    #[garde(range(min = 1))]
    pub output_limit: Option<u32>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<bool>")]
    #[garde(skip)]
    pub supports_tools: Option<bool>,
    /// Whether the model reads images and documents natively.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<bool>")]
    #[garde(skip)]
    pub supports_attachments: Option<bool>,
    /// Whether the model reads video natively.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<bool>")]
    #[garde(skip)]
    pub supports_video: Option<bool>,
    /// The exact types the model reads natively, when the source states
    /// them: `[]` for none, null when the source does not say
    /// (`models.md` § Accepted attachment types).
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Vec<FileExtension>>")]
    #[garde(skip)]
    pub accepted_extensions: Option<Vec<FileExtension>>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<bool>")]
    #[garde(skip)]
    pub supports_reasoning: Option<bool>,
    /// The thinking efforts the model offers, in the vendor's words.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Vec<String>>")]
    #[garde(skip)]
    pub supported_thinking_efforts: Option<Vec<String>>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub default_thinking_effort: Option<String>,
    /// Whether thinking can be turned off entirely; a transport that only
    /// levels thinking says no.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<bool>")]
    #[garde(skip)]
    pub can_disable_thinking: Option<bool>,
    /// The service tiers the model offers; empty for none.
    #[garde(dive)]
    pub service_tiers: Vec<ServiceTier>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub default_service_tier_id: Option<String>,
    /// Prices as the source reports them.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ModelCost>")]
    #[garde(skip)]
    pub cost: Option<ModelCost>,
}

/// A service tier a model offers. The product's Fast switch selects the tier
/// marked `fast` and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceTier {
    #[garde(length(utf16, min = 1))]
    pub id: String,
    #[garde(skip)]
    pub label: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub description: Option<String>,
    #[garde(skip)]
    pub fast: bool,
}

/// A model's prices in dollars per million tokens; null for a price the
/// source does not report.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCost {
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    pub input: Option<f64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    pub output: Option<f64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    pub cache_read: Option<f64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    pub cache_write: Option<f64>,
}
