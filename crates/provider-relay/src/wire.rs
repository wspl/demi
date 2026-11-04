//! The relay's wire: its routes and headers, and its own envelopes of the
//! provider contract's types (`crates/provider-common/src/contract.rs`) and
//! of the media calls (`crates/provider-common/src/media/channel.rs`), which
//! cross as they are.
//!
//! A socket's message is one binary frame of MessagePack, and a media call's
//! bodies are frames each of a 4-byte length and a MessagePack message
//! ([`media`]). Bytes cross as bytes. [`version`] is the digest of every
//! message's schema, so two servers talk only in the same shapes.

use std::num::NonZeroU32;
use std::sync::{Arc, LazyLock};

use bytes::Bytes;
use demi_provider_common::{
    InferenceItem, InferenceRequest, PromptCache, ProviderEvent, RequestBlock, RequestLimits, RequestPurpose,
    ToolDefinition,
};
use demi_shared_types::{ProviderModelList, ThinkingConfig};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

use crate::delta::Delta;

pub mod media;

/// `WS`: the socket of one borrowed session.
pub const SOCKET_ROUTE: &str = "/api/relay/socket";

/// `GET`: the lent provider as [`Lent`] describes it, as JSON.
pub const PROVIDER_ROUTE: &str = "/api/relay/provider";

/// The header of every relay request that names the borrower's [`version`].
pub const VERSION_HEADER: &str = "demi-relay-version";

/// The header of every relay request that names the borrowing server, as the
/// host of its public URL, which the lender shows beside the key it used.
pub const SERVER_HEADER: &str = "demi-relay-server";

/// The most bytes of one message: OpenAI's limit of one request's body, the
/// largest of the vendors', since a request the vendor would refuse need not
/// cross, and more than the largest file a media call answers.
pub const MESSAGE_BYTES: usize = RequestLimits::OPENAI.body_bytes.unwrap() as usize;

/// The wire's version: the SHA-256 of the JSON Schema of every message that
/// crosses, their descriptions left out, so that a change of a type that
/// crosses changes it and nothing else does.
pub fn version() -> &'static str {
    static VERSION: LazyLock<String> = LazyLock::new(|| {
        let mut schemas = vec![
            schemars::schema_for!(Lent).to_value(),
            schemars::schema_for!(ClientMessage).to_value(),
            schemars::schema_for!(ServerMessage).to_value(),
            schemars::schema_for!(demi_provider_common::media::channel::Call).to_value(),
            schemars::schema_for!(demi_provider_common::media::channel::Reply).to_value(),
        ];
        let mut digest = Sha256::new();
        for schema in &mut schemas {
            undescribed(schema);
            digest.update(serde_json::to_vec(schema).expect("a schema serializes"));
        }
        hex::encode(digest.finalize())
    });
    &VERSION
}

/// `schema` without what only describes it: a schema's `description` and
/// `title`, which are text, and never a property of that name, whose schema
/// is an object.
fn undescribed(schema: &mut serde_json::Value) {
    match schema {
        serde_json::Value::Object(object) => {
            for annotation in ["description", "title"] {
                if object.get(annotation).is_some_and(serde_json::Value::is_string) {
                    object.remove(annotation);
                }
            }
            object.values_mut().for_each(undescribed);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(undescribed),
        _ => {}
    }
}

/// `message` as MessagePack; none for a value its types do not let cross,
/// such as an account's event, which whoever sends it holds back first.
pub fn encoded(message: &impl Serialize) -> Option<Bytes> {
    match rmp_serde::to_vec_named(message) {
        Ok(bytes) => Some(bytes.into()),
        Err(error) => {
            tracing::error!(event = "relay.unencoded", %error, "a relay message holds what does not cross");
            None
        }
    }
}

/// The message `bytes` hold; none for what is no message of the wire.
pub fn decoded<T: DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    rmp_serde::from_slice(bytes).ok()
}

/// The answer to a relay request of another [`version`], with HTTP 409. A
/// borrower reads any 409 as another version: a lender of 0.1.5950 answered
/// its number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OtherVersion {
    pub version: String,
}

/// What [`PROVIDER_ROUTE`] answers: the provider's name as its lender calls
/// it, its catalog of the models the lender has on, and what one request of
/// each may hold. A provider the lender turned off lends nothing: its
/// catalog is empty, and its socket and media routes answer [`TURNED_OFF`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Lent {
    pub name: String,
    pub catalog: ProviderModelList,
    pub limits: Vec<ModelLimits>,
    pub turned_off: bool,
}

/// The status a relay route answers for a provider its lender turned off,
/// HTTP 423 Locked: the key still lends, once the provider is on again.
pub const TURNED_OFF: u16 = 423;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelLimits {
    pub model: String,
    pub limits: RequestLimits,
}

/// A message of the borrower's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum ClientMessage {
    /// Runs `request`; the run's events name `run`, which grows with each.
    Run { run: u64, request: RunRequest },
    /// Cancels the run `run`, which ends with no further event but its end.
    Cancel { run: u64 },
    /// The session's action ended.
    Idle,
}

/// A message of the lender's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum ServerMessage {
    /// An event the borrower may see (`crate::lend`).
    Event { run: u64, event: ProviderEvent },
    /// The run `run` ended: no event of it follows.
    End { run: u64 },
}

/// An [`InferenceRequest`] as it crosses: its data, with its long parts as
/// what changed since the session's last request (`crate::delta`). Its
/// cancellation is each end's own, and it names no account: the lender
/// keeps the session's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RunRequest {
    pub session_id: String,
    pub affinity: Option<String>,
    pub conversation: Option<String>,
    pub turn_id: String,
    pub request_id: String,
    pub model_id: String,
    pub output_limit: Option<NonZeroU32>,
    pub output_cap: Option<NonZeroU32>,
    /// None: the last request's.
    pub system_prompt: Option<String>,
    /// None: the last request's.
    pub tools: Option<Vec<ToolDefinition>>,
    pub items: Delta<InferenceItem>,
    pub blocks: Delta<RequestBlock>,
    pub thinking: Option<ThinkingConfig>,
    pub service_tier_id: Option<String>,
    pub prompt_cache: PromptCache,
    pub purpose: RequestPurpose,
    pub agent: Option<u64>,
}

impl RunRequest {
    /// `request` with the parts `crate::delta` gives.
    pub(crate) fn of(
        request: &InferenceRequest,
        system_prompt: Option<String>,
        tools: Option<Vec<ToolDefinition>>,
        items: Delta<InferenceItem>,
        blocks: Delta<RequestBlock>,
    ) -> Self {
        Self {
            session_id: request.session_id.clone(),
            affinity: request.affinity.clone(),
            conversation: request.conversation.clone(),
            turn_id: request.turn_id.clone(),
            request_id: request.request_id.clone(),
            model_id: request.model_id.clone(),
            output_limit: request.output_limit,
            output_cap: request.output_cap,
            system_prompt,
            tools,
            items,
            blocks,
            thinking: request.thinking.clone(),
            service_tier_id: request.service_tier_id.clone(),
            prompt_cache: request.prompt_cache,
            purpose: request.purpose,
            agent: request.agent,
        }
    }

    /// The contract's request, with the parts the delta resolved.
    pub(crate) fn into_request(
        self,
        system_prompt: String,
        tools: Arc<[ToolDefinition]>,
        items: Arc<[InferenceItem]>,
        blocks: Arc<[RequestBlock]>,
        cancel: CancellationToken,
    ) -> InferenceRequest {
        InferenceRequest {
            session_id: self.session_id,
            affinity: self.affinity,
            conversation: self.conversation,
            turn_id: self.turn_id,
            request_id: self.request_id,
            model_id: self.model_id,
            output_limit: self.output_limit,
            output_cap: self.output_cap,
            system_prompt,
            items,
            blocks,
            tools,
            thinking: self.thinking,
            service_tier_id: self.service_tier_id,
            prompt_cache: self.prompt_cache,
            cancel,
            account: None,
            purpose: self.purpose,
            agent: self.agent,
        }
    }
}
