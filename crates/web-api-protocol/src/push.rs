//! Push notifications: the backend's VAPID public key, a browser's subscription with the events it chose, and
//! the message each push carries to the page's service worker, which words and shows it.

use std::borrow::Cow;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use demi_shared_types::Nullable;
use garde::Validate;
use p256::PublicKey;
use p256::elliptic_curve::sec1::ToEncodedPoint as _;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::ids::ConversationId;
use crate::permissions::{PermissionCategory, PermissionHost, PermissionRequestId};

/// `GET /push/key`: the key the page subscribes with, the uncompressed
/// P-256 point in URL-safe base64 without padding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PushKey {
    pub public_key: String,
}

/// Where a push service takes the messages for one browser's subscription:
/// an `https` URL with a host, at most 2048 bytes. Whether its host is a
/// public one is the sender's to check, as it connects.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PushEndpoint(Url);

/// Why a text is not a [`PushEndpoint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the endpoint must be an https URL with a host, at most 2048 bytes")]
pub struct NotPushEndpoint;

impl PushEndpoint {
    pub fn url(&self) -> &Url {
        &self.0
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl TryFrom<String> for PushEndpoint {
    type Error = NotPushEndpoint;

    fn try_from(text: String) -> Result<Self, NotPushEndpoint> {
        if text.len() > 2048 {
            return Err(NotPushEndpoint);
        }
        let url = Url::parse(&text).map_err(|_| NotPushEndpoint)?;
        if url.scheme() != "https" || url.host().is_none() || !url.username().is_empty() || url.password().is_some() {
            return Err(NotPushEndpoint);
        }
        Ok(Self(url))
    }
}

impl From<PushEndpoint> for String {
    fn from(endpoint: PushEndpoint) -> Self {
        endpoint.0.into()
    }
}

impl JsonSchema for PushEndpoint {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "PushEndpoint".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "maxLength": 2048 })
    }
}

/// A browser's P-256 public key, which encrypts each message for it: in
/// JSON the uncompressed point in URL-safe base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BrowserKey(PublicKey);

/// Why a text is not a [`BrowserKey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("p256dh must be a P-256 public key in URL-safe base64")]
pub struct NotBrowserKey;

impl BrowserKey {
    pub fn key(&self) -> &PublicKey {
        &self.0
    }
}

impl From<PublicKey> for BrowserKey {
    fn from(key: PublicKey) -> Self {
        Self(key)
    }
}

impl TryFrom<String> for BrowserKey {
    type Error = NotBrowserKey;

    fn try_from(text: String) -> Result<Self, NotBrowserKey> {
        let bytes = URL_SAFE_NO_PAD
            .decode(text.trim_end_matches('='))
            .map_err(|_| NotBrowserKey)?;
        PublicKey::from_sec1_bytes(&bytes).map(Self).map_err(|_| NotBrowserKey)
    }
}

impl From<BrowserKey> for String {
    fn from(key: BrowserKey) -> Self {
        URL_SAFE_NO_PAD.encode(key.0.to_encoded_point(false).as_bytes())
    }
}

impl JsonSchema for BrowserKey {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "BrowserKey".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string" })
    }
}

/// A browser's 16-byte authentication secret: in JSON, URL-safe base64.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BrowserAuth([u8; 16]);

/// Why a text is not a [`BrowserAuth`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("auth must be 16 bytes in URL-safe base64")]
pub struct NotBrowserAuth;

impl BrowserAuth {
    pub fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub fn bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl std::fmt::Debug for BrowserAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrowserAuth(..)")
    }
}

impl TryFrom<String> for BrowserAuth {
    type Error = NotBrowserAuth;

    fn try_from(text: String) -> Result<Self, NotBrowserAuth> {
        let bytes = URL_SAFE_NO_PAD
            .decode(text.trim_end_matches('='))
            .map_err(|_| NotBrowserAuth)?;
        bytes.try_into().map(Self).map_err(|_| NotBrowserAuth)
    }
}

impl From<BrowserAuth> for String {
    fn from(auth: BrowserAuth) -> Self {
        URL_SAFE_NO_PAD.encode(auth.0)
    }
}

impl JsonSchema for BrowserAuth {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "BrowserAuth".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string" })
    }
}

/// The keys a browser's subscription gives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct PushSubscriptionKeys {
    pub p256dh: BrowserKey,
    pub auth: BrowserAuth,
}

/// What a browser chose to be notified of, the switches of Settings ›
/// Notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct PushEvents {
    pub turn_finishes: bool,
    pub turn_fails: bool,
    pub needs_permission: bool,
    pub agent_notifies: bool,
}

/// `PUT /push/subscription`: the browser's subscription for the caller's
/// session, which replaces what the session had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct PushSubscriptionBody {
    pub endpoint: PushEndpoint,
    pub keys: PushSubscriptionKeys,
    pub events: PushEvents,
}

/// What a push tells the page's service worker, by its `kind`, each of a
/// conversation, whose title is null while it has none. The worker words
/// it in the page's language and shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum PushMessage {
    /// A turn of the root ended with an answer and the conversation no
    /// longer runs; `answerStart` is the first [`ANSWER_START_CHARS`]
    /// characters (Unicode scalar values) of the answer's Markdown, null
    /// when the turn gave no text.
    TurnFinished {
        conversation_id: ConversationId,
        #[schemars(with = "Nullable<String>")]
        title: Option<String>,
        /// The block that ended the turn, which tags the notification.
        turn: String,
        #[schemars(with = "Nullable<String>")]
        answer_start: Option<String>,
    },
    /// A turn of the root ended with an error.
    TurnFailed {
        conversation_id: ConversationId,
        #[schemars(with = "Nullable<String>")]
        title: Option<String>,
        turn: String,
    },
    /// The conversation asks the user for what `ask` says.
    Permission {
        conversation_id: ConversationId,
        #[schemars(with = "Nullable<String>")]
        title: Option<String>,
        request: PermissionRequestId,
        ask: PushAsk,
    },
    /// The agent ran `demi notify send`: its title and body, as it wrote
    /// them.
    Agent {
        conversation_id: ConversationId,
        /// Tags the notification, one of its own.
        id: String,
        title: String,
        body: String,
    },
}

/// How much of a turn's last answer [`PushMessage::TurnFinished`] carries:
/// more than a notification shows, as its Markdown's syntax goes.
pub const ANSWER_START_CHARS: usize = 400;

/// What a permission request asks, as its card's title asks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum PushAsk {
    /// The grant of a category, or to reach a host.
    Permission {
        categories: Vec<PermissionCategory>,
        #[schemars(with = "Nullable<PermissionHost>")]
        host: Option<PermissionHost>,
    },
    /// To save the credential `name`.
    Credential { name: String },
    /// To use the saved credential `name`.
    Allowance { name: String },
}

impl PushMessage {
    /// The conversation it is of.
    pub fn conversation(&self) -> &ConversationId {
        match self {
            Self::TurnFinished { conversation_id, .. }
            | Self::TurnFailed { conversation_id, .. }
            | Self::Permission { conversation_id, .. }
            | Self::Agent { conversation_id, .. } => conversation_id,
        }
    }

    /// Whether a browser that chose `events` is notified of it.
    pub fn chosen_by(&self, events: &PushEvents) -> bool {
        match self {
            Self::TurnFinished { .. } => events.turn_finishes,
            Self::TurnFailed { .. } => events.turn_fails,
            Self::Permission { .. } => events.needs_permission,
            Self::Agent { .. } => events.agent_notifies,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cost: a few parses; under a millisecond.

    #[test]
    fn only_an_https_url_with_a_host_is_an_endpoint() {
        for refused in ["http://push.example/x", "ftp://push.example/x", "https://user:pw@push.example/x", "not a url"] {
            assert!(PushEndpoint::try_from(refused.to_owned()).is_err(), "{refused}");
        }
    }
}
