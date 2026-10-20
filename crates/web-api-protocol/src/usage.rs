//! The usage ledger's totals (`crates/backend-database/src/usage.rs`): one
//! group per provider entry and model, in the order each pair was first
//! used, with the number of requests and the sum of each token count.

use demi_conversation_socket_protocol::{AgentCost, CostCall, CostTotals};
use demi_shared_types::MAX_SAFE_INTEGER;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::UserId;
use crate::text::EmailAddress;

/// `GET /usage`: the caller's totals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageTotals {
    pub totals: Vec<UsageGroup>,
}

/// The requests of one provider entry and model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroup {
    pub provider_id: String,
    pub model_id: String,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub requests: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub input_tokens: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub output_tokens: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub cache_read_tokens: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub cache_write_tokens: u64,
}

/// `GET /usage/instance`: on a shared instance, every account's totals, for
/// an administrator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InstanceUsage {
    pub users: Vec<UserUsage>,
}

/// One account's totals, the accounts in the order they were created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserUsage {
    pub user_id: UserId,
    pub email: EmailAddress,
    pub totals: Vec<UsageGroup>,
}

/// `?agent=` of the cost read: the agent, by its number, whose own cost the
/// read answers; absent, the whole conversation's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostQuery {
    #[serde(default)]
    pub agent: Option<u64>,
}

/// `GET /conversations/:id/cost`: the conversation's cost, computed from its
/// ledger rows when read (`crates/backend-database/src/usage.rs`); with
/// `?agent=`, that agent's own in the same shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct ConversationCost {
    /// The number of the conversation's ledger rows the cost sums, which a
    /// `cost` frame continues.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub rows: u64,
    /// Every row of the conversation: every agent, compactions and titles
    /// included; or the agent's rows.
    #[garde(dive)]
    pub totals: CostTotals,
    /// Every agent with a priced call, by number, the main conversation
    /// first; or the agent alone, once it made one.
    #[garde(dive)]
    pub agents: Vec<AgentCost>,
    /// The main conversation's turn calls that the vendor reported counts
    /// for, in the order they started; or the agent's.
    #[garde(dive)]
    pub calls: Vec<CostCall>,
}
