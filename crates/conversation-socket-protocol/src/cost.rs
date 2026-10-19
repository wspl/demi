//! A conversation's cost as the context card shows it
//! (`usage-and-quota.md` § A conversation's cost): the token and price
//! totals of its ledger rows, each agent's price and the main
//! conversation's turn calls with their cache misses. `GET
//! /api/conversations/:id/cost` answers them whole, and the `cost` frame
//! carries what one new row adds to them.

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

/// One agent's price: the main conversation's, agent 0, or a subagent's,
/// with the profile it was started with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct AgentCost {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub agent: u64,
    /// The profile the subagent was started with; null for none and for the
    /// main conversation.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub profile: Option<String>,
    #[garde(skip)]
    pub price: f64,
}

/// One turn call of the main conversation, as the cache chart plots it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CostCall {
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
    /// The main conversation answered a compaction between this call and
    /// the previous one, so the call starts a new prompt.
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
