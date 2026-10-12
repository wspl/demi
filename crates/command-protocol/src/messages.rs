//! The messages stream (`native-runtime.md` § Messages to the plugin): what
//! a service tells the plugin of its package about the state it keeps for a
//! conversation, which the runner forwards to the backend, and the runner's
//! answer once its connection took a message.

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
