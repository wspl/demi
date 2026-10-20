//! The messages stream: what a service tells the plugin of its package about
//! the state it keeps for a conversation, which the runner forwards to the
//! backend, and the runner's answer once its connection took a message.
//!
//! The service tells when its state changes; nobody asks it, so an agent that
//! runs `rg` a thousand times moves no message. The runner answers a message
//! once its connection took it, and a service has a bounded number
//! unanswered, so one that writes faster than the backend reads waits in its
//! send and nothing piles up between. A service that would say the same thing
//! twice says it once: the browser keeps the conversations whose tabs changed
//! since its last message, so a burst of title and favicon changes costs one.
//!
//! ```text
//! Service                         Runner                         Backend
//!   |<-- POST /v1/messages ----------|  opened after GET /v1/info    |
//!   |--- {id 4, c1, {"tabs":..}} --->|--- package_message ---------->|--> plugin
//!   |<-- {id 4} ---------------------|  taken by the connection      |
//! ```
//!
//! The backend accepts a message only for a conversation of the device's user
//! that reaches the device, as its primary Host or an attached one, and logs
//! and drops any other; it hands one connection's messages on one after
//! another.
//!
//! When the state a service kept for a conversation ends without the service
//! saying so, whoever sees it end says it in the service's place, and the
//! plugin receives a `package_lost` request for the conversation: the runner
//! sends it with the package when the service ends without releasing its
//! conversations, as a crash does, and the backend sends it for every package
//! when the runner starts anew (its hello names another instance than the
//! device's last) or a Cloud stops. It goes to the plugin whose commands bind
//! the package, when that plugin has panel tabs in a conversation that
//! reaches the device, once for each such conversation: a plugin that shows
//! nothing of a conversation has nothing to correct.

use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use super::invocation::conversation_name;
use super::{MAX_RECORD_BYTES, ProtocolError};

/// One message for the plugin, which the service writes as one standard
/// output record. `id` is the service's own, unique among its messages in
/// flight; what `message` says is the package's and its plugin's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginMessage {
    #[garde(skip)]
    pub id: u64,
    #[garde(custom(conversation_name))]
    pub conversation: String,
    #[garde(skip)]
    pub message: serde_json::Value,
}

/// The answer to message `id`, one input chunk: none once the runner's
/// connection took it, or why it did not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageAnswer {
    #[garde(skip)]
    pub id: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(length(min = 1)))]
    pub error: Option<String>,
}

impl PluginMessage {
    /// Checks the message's fields and that it fits one record.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self).map_err(ProtocolError::from)?;
        if serde_json::to_vec(self)?.len() > MAX_RECORD_BYTES {
            return Err(ProtocolError::TooLarge);
        }
        Ok(())
    }
}

impl MessageAnswer {
    pub fn new(id: u64, result: Result<(), String>) -> Self {
        Self {
            id,
            error: result.err(),
        }
    }

    /// The answer's outcome.
    pub fn outcome(&self) -> Result<Result<(), String>, ProtocolError> {
        garde::Validate::validate(self).map_err(ProtocolError::from)?;
        Ok(match &self.error {
            None => Ok(()),
            Some(error) => Err(error.clone()),
        })
    }
}
