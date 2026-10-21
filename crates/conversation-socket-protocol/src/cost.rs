//! A conversation's cost as the context card shows it: each agent's kept
//! totals with its metadata, the conversation's with its own, and each
//! agent's cache chart, which comes with its node
//! (`crates/backend-database/src/usage.rs`).

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
/// how many turn calls it made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct AgentCost {
    #[garde(dive)]
    pub totals: CostTotals,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub calls: u64,
}

/// The cache hit rate of an agent's recent turn calls and the chart of it, as
/// the backend counts them from its usage ledger
/// (`crates/backend-database/src/usage.rs`) and a context card draws them.
///
/// The window is the agent's latest calls, at most 400, which the points
/// share out: each point stands for `size` consecutive calls counted from
/// the agent's call 1, at most 40 points, and the window starts at a point's
/// first call, so a new call changes only the last point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CacheChart {
    /// The share of the window's prompt tokens the vendor read from its
    /// cache; null without a call that sent a prompt.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    #[garde(skip)]
    pub rate: Option<f64>,
    /// The calls each point stands for; the last point may hold fewer.
    #[garde(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub size: u64,
    /// The points, oldest first; none before the agent's fourth call, when
    /// a line says nothing a user can read and the rate alone shows.
    #[garde(dive)]
    pub points: Vec<ChartPoint>,
}

impl Default for CacheChart {
    /// The chart of an agent that made no turn call.
    fn default() -> Self {
        Self {
            rate: None,
            size: 1,
            points: Vec::new(),
        }
    }
}

/// One point of a cache chart: consecutive turn calls of an agent, or one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct ChartPoint {
    /// The number of its first call among its agent's turn calls, from 1.
    #[garde(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub first: u64,
    /// The number of its last call; `first` for a point of one call.
    #[garde(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub last: u64,
    /// The share of its calls' prompt tokens read from the cache; 0 when
    /// they sent none.
    #[garde(skip)]
    pub rate: f64,
    /// When its first call started.
    #[garde(skip)]
    pub from: Timestamp,
    /// When its last call started.
    #[garde(skip)]
    pub to: Timestamp,
    /// The causes of its calls' cache misses, each once, in the order they
    /// first came.
    #[garde(skip)]
    pub misses: Vec<CacheMiss>,
    /// Its agent answered a compaction before one of its calls, so a new
    /// prompt starts within it.
    #[garde(skip)]
    pub after_compaction: bool,
    /// What its calls would cost on the vendors' APIs, in dollars; null
    /// when none of them has a price.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    #[garde(skip)]
    pub price: Option<f64>,
    /// For a point of one call, the tokens the call sent uncached or wrote
    /// to the cache, which its tip names when it missed; null for a point
    /// of several.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u64>")]
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub rewritten: Option<u64>,
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
