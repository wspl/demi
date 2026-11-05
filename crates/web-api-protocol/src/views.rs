//! View links: a tab of a conversation's browser, or the conversation
//! itself, shown to whoever holds the link, which an integration makes. Its
//! page reads what the link shows here, then opens the tab's stream or the
//! conversation's page connection.

use demi_conversation_lane_protocol::JobModel;
use demi_shared_types::{Nullable, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::conversations::ConversationDetail;

/// `GET /views/:token`: what a view link's page reads before it opens its
/// socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ViewLink {
    /// One tab of the conversation's browser.
    Tab {
        /// The tab, such as `t3`.
        tab: String,
        /// The page only watches.
        read_only: bool,
        /// None for a link that opens until it is removed.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<Timestamp>")]
        expires_at: Option<Timestamp>,
    },
    /// The conversation itself, which the page only watches.
    Conversation {
        /// None for a link that opens until it is removed.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<Timestamp>")]
        expires_at: Option<Timestamp>,
    },
}

/// `GET /views/:token/conversation`: the conversation as its own page reads
/// it, and what its root agent runs on in the names the model menu shows,
/// which the page has no catalog to find.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewedConversation {
    pub conversation: ConversationDetail,
    /// None for a conversation with no model yet.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<JobModel>")]
    pub runs_on: Option<JobModel>,
}
