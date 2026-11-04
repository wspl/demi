//! View links: one tab of a conversation's browser, shown to whoever holds
//! the link, which an integration makes. Its page reads what the link shows
//! here, then opens the tab's stream.

use demi_shared_types::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `GET /views/:token`: what a view link's page reads before it opens its
/// socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewLink {
    /// The tab the link shows, such as `t3`.
    pub tab: String,
    /// The page only watches.
    pub read_only: bool,
    pub expires_at: Timestamp,
}
