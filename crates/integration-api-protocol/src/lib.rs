//! The integration API's contract: every body its routes take and answer and
//! every message of its socket, under `/v1`
//! (`crates/backend-http/src/integration_edge.rs`).
//!
//! An integration is a program that builds an agent product on Demi: it makes
//! Clouds, conversations on them, sends messages and hears every change of
//! its conversations. Its requests carry its token as a bearer token.
//!
//! The types here are the integration's, apart from the web app's
//! (`web-api-protocol`): no page-only shape, such as a follow or a page's
//! read state, is in them, so a change to a page breaks no integration. They
//! reuse the identifiers, the error body and the transcript's blocks and
//! patches, which are the domain's own. The OpenAPI document is generated
//! from them.
//!
//! A body the backend receives refuses unknown fields.

use demi_conversation_lane_protocol::{TranscriptPatch, Version};
use demi_shared_types::{Block, IntegrationCapabilities, NodeId, Timestamp};
use demi_web_api_protocol::cloud::CloudState;
use demi_web_api_protocol::conversations::ConversationStatus;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, IntegrationId};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

pub use demi_web_api_protocol::error::{ErrorBody, ErrorCode};
pub use openapi::openapi;

mod openapi;

/// The prefix of every route of the integration API.
pub const PREFIX: &str = "/v1";

/// The longest key an integration gives a Cloud or a conversation, in
/// characters: a key names a thing of the integration's, such as a chat's and
/// a thread's ids as JSON, never a document.
pub const KEY_CHARS: usize = 1024;

/// `GET /integration`: the integration as it is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Integration {
    pub id: IntegrationId,
    pub name: String,
    pub capabilities: IntegrationCapabilities,
}

/// `PATCH /integration`: a change of its capabilities.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntegrationPatch {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "IntegrationCapabilities")]
    #[garde(skip)]
    pub capabilities: Option<IntegrationCapabilities>,
}

/// One of the integration's Clouds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Cloud {
    pub id: DeviceId,
    /// The key the integration made it with; null for none.
    pub key: Option<String>,
    pub state: CloudState,
}

/// `POST /clouds`: a Cloud, or with `key` the one that has it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateCloud {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 1024))]
    #[garde(inner(length(chars, min = 1, max = KEY_CHARS)))]
    pub key: Option<String>,
}

/// `GET /clouds`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloudList {
    pub clouds: Vec<Cloud>,
}

/// A model a conversation runs on: one of the provider entries and models of
/// `GET /models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct ModelChoice {
    pub provider_id: String,
    pub model_id: String,
}

/// One of the integration's conversations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: ConversationId,
    /// The key the integration made it with; null for none.
    pub key: Option<String>,
    /// The Cloud its main agent works on; null for one whose Cloud is gone.
    pub cloud: Option<DeviceId>,
    pub title: String,
    pub status: ConversationStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// `POST /conversations`: a conversation on `cloud`, or with `key` the one
/// that has it, as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateConversation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 1024))]
    #[garde(inner(length(chars, min = 1, max = KEY_CHARS)))]
    pub key: Option<String>,
    #[garde(skip)]
    pub cloud: DeviceId,
    /// The workflow its agents spawn subagents by, in the format of a
    /// project's workflow file; none for no subagent profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub workflow: Option<String>,
    /// Its model, one of `GET /models`.
    #[garde(dive)]
    pub model: ModelChoice,
}

/// `PATCH /conversations/:id`: moves it to another Cloud, which its main
/// agent works on from its next command, or renames it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub cloud: Option<DeviceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    #[garde(inner(length(chars, min = 1)))]
    pub title: Option<String>,
}

/// `GET /conversations`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationList {
    pub conversations: Vec<Conversation>,
}

/// `POST /conversations/:id/messages`: a message. While a turn runs it waits
/// for the turn's end, or with `steer` joins the running turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SendMessage {
    /// The id it takes, which a retry repeats so nothing is sent twice; one
    /// is made when none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 64))]
    #[garde(inner(length(chars, min = 1, max = 64)))]
    pub id: Option<String>,
    #[schemars(length(min = 1))]
    #[garde(length(chars, min = 1))]
    pub text: String,
    #[serde(default)]
    #[garde(skip)]
    pub steer: bool,
}

/// What a sent message became.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageAnswer {
    pub id: String,
}

/// An agent of a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: NodeId,
    /// 0 for the main agent.
    pub number: u64,
    /// The agent that spawned it; null for the main agent.
    pub parent: Option<NodeId>,
    /// Its spawn's description; empty for the main agent.
    pub description: String,
    /// Whether its turn runs.
    pub running: bool,
    /// The Cloud it works on when that is not the conversation's; null on
    /// the conversation's.
    pub cloud: Option<DeviceId>,
}

/// `GET /conversations/:id`: the conversation with its agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDetail {
    pub conversation: Conversation,
    pub agents: Vec<Agent>,
}

/// `GET /conversations/:id/agents/:agent/transcript`: an agent's latest
/// blocks on its current path, at `version`, which the socket's patches of
/// the agent follow; null for an agent with no transcript yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub version: Option<Version>,
    pub blocks: Vec<Block>,
}

/// What the socket (`GET /v1/socket`) sends: every change of each of the
/// integration's conversations, as it happens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Event {
    /// An agent's transcript changed: its patches, at the version they lead
    /// to, in the order they happened.
    Transcript {
        conversation: ConversationId,
        agent: NodeId,
        version: Version,
        patches: Vec<TranscriptPatch>,
    },
    /// An agent started, ended, or its state or Cloud changed.
    Agent {
        conversation: ConversationId,
        agent: Agent,
    },
    /// The conversation's status changed. A turn has ended when it leaves
    /// `running` and `compacting`: `completed`, `stopped` or `error` says
    /// how.
    Conversation {
        conversation: ConversationId,
        status: ConversationStatus,
    },
}

/// A model a conversation may run on: one of its integration's user's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub provider_id: String,
    /// The provider entry's name.
    pub provider: String,
    pub model_id: String,
    /// The model's name.
    pub name: String,
}

/// `GET /models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelList {
    pub models: Vec<Model>,
}
