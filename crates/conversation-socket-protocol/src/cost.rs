//! A conversation's cost as the context card shows it: each agent's kept
//! totals with its metadata, the conversation's with its own, and an agent's
//! turn calls with their cache misses, which the cache chart reads when it
//! opens (`crates/backend-database/src/usage.rs`).

use demi_shared_types::{MAX_SAFE_INTEGER, Nullable, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The tokens of the rows a figure sums, by kind, and their prices.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CostTotals {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub input_tokens: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub cache_write_tokens: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub cache_read_tokens: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub output_tokens: u64,
    #[garde(skip)]
    pub price: CostPrice,
}

/// What the rows would cost on the vendors' APIs, in dollars, in all and by
/// kind of token; each null when no row it sums has a price. The kinds add
/// up to less than the total by the rows priced at the vendor's own cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CostPrice {
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    pub total: Option<f64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    pub input: Option<f64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    pub cache_write: Option<f64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    pub cache_read: Option<f64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    pub output: Option<f64>,
}

/// What one agent's rows cost, kept as they are written: their totals, and
/// how many turn calls it made, which tells a page holding its calls whether
/// it holds them all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct AgentCost {
    #[garde(dive)]
    pub totals: CostTotals,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub calls: u64,
}

/// One turn call of an agent, as the cache chart plots it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CostCall {
    /// Its number among its agent's turn calls, from 1.
    #[garde(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub number: u64,
    #[garde(skip)]
    pub started_at: Timestamp,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub input_tokens: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub cache_write_tokens: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub cache_read_tokens: u64,
    /// Null when the call has no price.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    #[garde(skip)]
    pub price: Option<f64>,
    /// Its agent answered a compaction between this call and the previous
    /// one, so the call starts a new prompt.
    #[garde(skip)]
    pub after_compaction: bool,
    /// Why the call wrote again what an earlier call had cached; null when
    /// it is no cache miss.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<CacheMiss>")]
    #[garde(skip)]
    pub miss: Option<CacheMiss>,
}

/// Why a call missed the cache, the first of these that applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CacheMiss {
    /// The call's model is not the previous call's.
    ModelChanged,
    /// The tool definitions changed.
    ToolsChanged,
    /// The system prompt changed, as when an instructions file changed.
    SystemPromptChanged,
    /// Longer passed since the previous call than the vendor keeps a prompt.
    CacheExpired,
    /// None of the above: the vendor dropped or never stored the prompt.
    NotCached,
}

serde_plain::derive_display_from_serialize!(CacheMiss);
serde_plain::derive_fromstr_from_deserialize!(CacheMiss);
