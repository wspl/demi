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

use std::num::NonZeroU32;

use demi_command_declarations::{NativeOperation, Node};
use demi_conversation_lane_protocol::{JobModel, JobPhase, TranscriptPatch, Version};
use demi_host_interface::{PortRequest, PortResponse};
use demi_shared_types::{
    Block, BlockId, FileExtension, IntegrationCapabilities, NodeId, ProviderErrorDiagnostics, SessionPhase, Timestamp,
};
use demi_web_api_protocol::cloud::CloudState;
use demi_web_api_protocol::conversations::ConversationStatus;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, IntegrationId};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

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
    /// The shell script each of its Clouds runs as `demi` in `~` once its
    /// system is new: at its first use, and after every reset. Null for
    /// none.
    pub init_script: Option<String>,
    /// Where each of its Clouds mounts its shared folder: `~/...`, or a
    /// top-level directory such as `/data`.
    pub shared_path: String,
}

/// `PATCH /integration`: a change of its settings; a field left out stays.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntegrationPatch {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "IntegrationCapabilities")]
    #[garde(skip)]
    pub capabilities: Option<IntegrationCapabilities>,
    /// A new init script, or null for none. A Cloud runs it when it is new or
    /// reset; one whose system was made already keeps that system.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Option<String>")]
    #[garde(skip)]
    pub init_script: Option<Option<String>>,
    /// A new mount path of the shared folder, which each Cloud takes at
    /// its next start.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub shared_path: Option<String>,
}

/// One of the integration's Clouds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Cloud {
    pub id: DeviceId,
    /// The key the integration made it with; null for none.
    pub key: Option<String>,
    pub state: CloudState,
    /// Its run of the integration's init script; null while the integration
    /// has none, or the Cloud's system was made while it had none.
    pub init: Option<CloudInit>,
}

/// A Cloud's run of its integration's init script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloudInit {
    pub state: InitState,
    /// How its last run ended; null before one ended.
    pub exit_code: Option<i32>,
    /// Its last run's output, standard output and error as they came, kept
    /// as a command's is: whole, or its first and last 8 MiB around a line
    /// that counts the bytes left out; null before one ended.
    pub output: Option<String>,
}

/// Where a Cloud's run of the init script is. A Cloud whose run failed
/// still serves its conversations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum InitState {
    /// It runs at the Cloud's next use: the Cloud is new, or was reset.
    Pending,
    /// It runs; the Cloud's conversations' commands wait for it.
    Running,
    Done,
    Failed,
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
    /// A thinking effort the model lists, or `off`; the model's first when
    /// none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// A service tier the model lists; the vendor's default when none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
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
/// agent works on from its next command, renames it, or changes its model
/// or its workflow, from its main agent's next request.
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
    /// Its model: a subagent keeps the one its spawn gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub model: Option<ModelChoice>,
    /// Its workflow, in the format of a project's workflow file; null for
    /// none.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Option<String>")]
    #[garde(skip)]
    pub workflow: Option<Option<String>>,
}

/// `GET /conversations?key=&limit=&after=`: a page of the integration's
/// conversations, the latest made first; with `key` the one of that key,
/// or none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(inner(length(chars, min = 1, max = KEY_CHARS)))]
    pub key: Option<String>,
    /// How many, 1 to 500; 100 when none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 500))]
    #[garde(inner(range(min = 1, max = 500)))]
    pub limit: Option<u32>,
    /// The `next` of the page before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub after: Option<String>,
}

/// `GET /conversations`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationList {
    pub conversations: Vec<Conversation>,
    /// The `after` of the next page; null on the last.
    pub next: Option<String>,
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
    /// Files it carries, which the agent finds on its Cloud.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(skip)]
    pub attachments: Vec<MessageAttachment>,
}

/// A file a message carries: an upload of `POST /attachments`, and the name
/// it is written under on the Cloud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageAttachment {
    pub id: String,
    /// A file name: no `/`, `\` or NUL, and neither `.` nor `..`.
    pub name: String,
}

/// `POST /attachments`: a file in the user's blobs, which a message carries
/// by its id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    /// The media type read from its bytes.
    pub media_type: String,
    pub size_bytes: u64,
    /// Its SHA-256 in hexadecimal, which `GET /blobs/{sha256}` reads it by.
    pub sha256: String,
}

/// A file a conversation holds, which a message or a reply names as `a3`:
/// an agent's attachment to its reply, or a file a message carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationAttachment {
    /// Its number in the conversation, such as `a3`.
    pub id: String,
    pub name: String,
    pub media_type: String,
    pub size_bytes: u64,
    /// Its SHA-256 in hexadecimal, which `GET /blobs/{sha256}` reads it by.
    pub sha256: String,
}

/// `POST /attachments?name=`: the name of the file whose bytes the body
/// holds, sent with its media type as `Content-Type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentQuery {
    #[garde(length(chars, min = 1, max = 255))]
    pub name: String,
}

/// `GET /usage`: the integration's usage between `from` and `to`, grouped by
/// `by`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageQuery {
    /// The period's start, inclusive; none for the first request.
    #[serde(default)]
    #[garde(skip)]
    pub from: Option<Timestamp>,
    /// The period's end, exclusive; none for now.
    #[serde(default)]
    #[garde(skip)]
    pub to: Option<Timestamp>,
    #[garde(skip)]
    pub by: UsageGrouping,
}

/// What `GET /usage` groups its rows by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UsageGrouping {
    Cloud,
    Conversation,
    Model,
}

/// The integration's usage, a row per group, most expensive first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub rows: Vec<UsageRow>,
}

/// One group's usage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    /// The group: a Cloud's or a conversation's id, or a model's id; null
    /// for requests of no Cloud, such as a title's.
    pub key: Option<String>,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// What the requests would cost on their vendors' APIs, in dollars;
    /// null when a model of them has no price.
    pub api_cost: Option<f64>,
}

/// What a sent message became.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageAnswer {
    pub id: String,
}

/// An agent of a conversation, as the Agents window shows it.
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
    /// Where its session is: `idle`, `running`, `compacting`, or
    /// `compacting_idle` for a compaction that no one asked for.
    pub phase: SessionPhase,
    /// The Cloud it works on when that is not the conversation's; null on
    /// the conversation's.
    pub cloud: Option<DeviceId>,
    /// A subagent's round; null for the main agent.
    pub round: Option<Round>,
    /// The messages that wait for its turn to end, in the order they run.
    pub queue: Vec<Waiting>,
    /// The steers it took that wait for its next step, in order.
    pub pending_steers: Vec<Waiting>,
}

/// A subagent's round: on what it runs, and where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Round {
    /// 1 for the spawn's, one more at each resume.
    pub number: u64,
    /// The profile's name; null for the inherit profile.
    pub profile: Option<String>,
    /// The provider entry's id and name, the model's name, and the effort.
    pub model: JobModel,
    /// `running`, or how it closed: `completed`, `aborted` or `error`.
    pub state: JobPhase,
    pub started_at: Timestamp,
    /// Null while it runs.
    pub ended_at: Option<Timestamp>,
}

/// A message that waits in an agent's queue or as its pending steer: its
/// id, the one it was sent with, and its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub id: String,
    pub text: String,
}

/// `POST /conversations/:id/stop`: which agents stop; every agent when
/// none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StopRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    pub scope: Option<StopScope>,
}

/// What a stop stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StopScope {
    /// The main agent's turn, as the web app's Stop.
    Main,
    /// Every subagent the main agent runs.
    Subagents,
}

/// `POST /conversations/:id/agents/:agent/messages`: a message to a
/// subagent, which it takes into its turn, starts a turn with, or is
/// resumed with. It takes no id of the program's: a closed subagent is
/// resumed as `demi agent resume` resumes one, which names no message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentMessage {
    #[schemars(length(min = 1))]
    #[garde(length(chars, min = 1))]
    pub text: String,
}

/// `GET /conversations/:id/agents/:agent/transcript`'s query: a page around
/// a place, as the web app reads one; the latest page when it names none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct TranscriptQuery {
    /// The page that ends before the block at this index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<u32>,
    /// The page that starts after the block at this index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<u32>,
    /// The page around this block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub around: Option<BlockId>,
    /// The block the page holds at `before` or `after`, which a page that
    /// moved since answers 409 for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<BlockId>,
}

/// `POST /conversations/:id/fork`: a new conversation of the integration's
/// with the transcript up to `blockId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForkRequest {
    #[garde(skip)]
    pub block_id: BlockId,
    /// The new conversation's key; none for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = KEY_CHARS))]
    #[garde(inner(length(chars, min = 1, max = KEY_CHARS)))]
    pub key: Option<String>,
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
    /// The index of the page's first block in the agent's path.
    pub start: u32,
    pub blocks: Vec<Block>,
    /// Whether the page holds the path's first block, and its last.
    pub at_start: bool,
    pub at_end: bool,
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
    /// The main agent retries a transient provider failure after `delayMs`,
    /// as the web app hears it. It is not kept: the turn's outcome is.
    RetryScheduled {
        conversation: ConversationId,
        attempt: u32,
        delay_ms: u64,
        code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diagnostics: Option<ProviderErrorDiagnostics>,
    },
    /// The conversation's status changed. A turn has ended when it leaves
    /// `running` and `compacting`: `completed`, `stopped` or `error` says
    /// how.
    Conversation {
        conversation: ConversationId,
        status: ConversationStatus,
    },
    /// What a `declare` this socket sent became: the plugin it serves from
    /// now on, or why it was refused, which changed nothing.
    Declared {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refused: Option<String>,
    },
    /// A request of the integration's plugin, to this socket, which served
    /// the plugin when it was sent: a `reply` of the same `id` answers it.
    Request { id: u64, request: PluginRequest },
    /// The answer to the `port` message `port` of the request `id`.
    Answer {
        id: u64,
        port: u64,
        response: PortResponse,
    },
    /// The agent stopped the call of the request `id`: its reply is not read.
    Cancel { id: u64 },
}

/// A request of an integration's plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum PluginRequest {
    /// An agent ran one of the plugin's commands. The command's output,
    /// input and wait go through `port` messages; `reply` ends it with its
    /// exit code.
    Command {
        conversation: ConversationId,
        key: Option<String>,
        /// The number of the agent that ran it, as `Agent.number` gives it.
        agent_number: Option<u64>,
        /// The Cloud it ran on.
        cloud: String,
        /// The command's path, its root first, such as `["chatbot", "post"]`.
        path: Vec<String>,
        /// The command line as the agent wrote it.
        argv: Vec<String>,
        /// The arguments, valid against the command's input.
        args: serde_json::Map<String, serde_json::Value>,
        /// Whether the agent passed `--json`.
        json: bool,
        /// The directory it ran in.
        cwd: String,
        /// Whether it has a standard input to read with `read_stdin`.
        stdin: bool,
    },
    /// An agent is about to ask its model: the plugin, which declared
    /// itself a context source, may add a context block. `seen` holds the
    /// text of its own blocks the model receives, oldest first.
    Context {
        conversation: ConversationId,
        key: Option<String>,
        agent: NodeId,
        seen: Vec<String>,
    },
}

/// What the socket takes from the integration's program.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum ClientMessage {
    /// Serve the integration's plugin from now on: its command trees, each
    /// a root of its own whose leaves are `rpc` leaves, and whether it is a
    /// context source. The last declaration is the integration's.
    Declare {
        commands: Vec<Node<NativeOperation>>,
        #[serde(default)]
        context: bool,
    },
    /// The reply to the request `id`.
    Reply { id: u64, reply: PluginReply },
    /// A message of the call of the command request `id` to its port: its
    /// standard output or error, a read of its input, or a wait for the
    /// user; `port` numbers it, which its `answer` names.
    Port {
        id: u64,
        port: u64,
        request: PortRequest,
    },
}

/// A reply to a plugin request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum PluginReply {
    /// A command ended with `code`.
    Exit { code: u8 },
    /// The context block to add; none for none.
    Context { text: Option<String> },
    /// The request failed, as `message` says.
    Failed { message: String },
}

/// What an integration declares, as storage keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Declaration {
    pub commands: Vec<Node<NativeOperation>>,
    pub context: bool,
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
    /// Its context window and the most it writes in one answer, in tokens;
    /// null where the catalog does not know.
    pub context_window: Option<u32>,
    pub output_limit: Option<u32>,
    /// The thinking efforts it offers, in the vendor's words; empty for none.
    pub efforts: Vec<String>,
    /// Whether thinking can be turned off, with the effort `off`.
    pub can_disable_thinking: bool,
    /// The service tiers it offers; empty for none.
    pub service_tiers: Vec<String>,
    /// The file types it reads natively; null where the catalog does not
    /// say.
    pub file_types: Option<Vec<FileExtension>>,
}

/// `GET /models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelList {
    pub models: Vec<Model>,
}
