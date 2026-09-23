//! The usage ledger's totals (`usage-and-quota.md` § Usage ledger): one
//! group per provider entry and model, in the order each pair was first
//! used, with the number of requests and the sum of each token count.

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
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
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
