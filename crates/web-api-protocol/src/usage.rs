//! The usage ledger's totals (`crates/backend-database/src/usage.rs`): one
//! group per provider entry and model, in the order each pair was first
//! used, with the number of requests and the sum of each token count.

use demi_conversation_socket_protocol::CostCall;
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

/// `?after=` of `GET /conversations/:id/agents/:node/calls`: the number of
/// the agent's last turn call the page holds; absent, it holds none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallsQuery {
    #[serde(default)]
    pub after: Option<u64>,
}

/// `GET /conversations/:id/agents/:node/calls`: the agent's turn calls after
/// the one the page names, which a context card's cache chart plots, read
/// when the card opens; the recent ones only, numbered so a page appends
/// them to what it holds (`crates/backend-database/src/usage.rs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct AgentCalls {
    #[garde(dive)]
    pub calls: Vec<CostCall>,
}
