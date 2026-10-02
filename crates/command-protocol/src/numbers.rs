//! The numbers stream (`native-runtime.md` § Conversation numbers): a
//! service's requests for numbers of a conversation's sequence, which the
//! runner forwards to the backend, and their answers.

use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use super::ProtocolError;
use super::invocation::conversation_name;

/// The most numbers one request reserves.
pub const MAX_NUMBERS: u32 = 16;

/// A sequence of the conversation that a native service draws numbers from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceSequence {
    /// The conversation browser's tabs (`browser.md` § One tab registry).
    Tab,
}

/// The metadata that opens a stream the runner answers a service's requests
/// on, the numbers stream or the artifacts stream, which carries nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct StreamOpen {}

/// One request for `count` numbers of the conversation's `sequence`; the
/// service writes each as one standard output record. `id` is the service's
/// own, unique among its requests in flight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NumbersRequest {
    #[garde(skip)]
    pub id: u64,
    #[garde(custom(conversation_name))]
    pub conversation: String,
    #[garde(skip)]
    pub sequence: ServiceSequence,
    #[garde(range(min = 1, max = MAX_NUMBERS))]
    pub count: u32,
}

/// The answer to request `id`, one input chunk: the first of its `count`
/// consecutive numbers, or why there are none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NumbersAnswer {
    #[garde(skip)]
    pub id: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(range(min = 1)))]
    pub first: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(length(min = 1)))]
    pub error: Option<String>,
}

impl NumbersRequest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self).map_err(ProtocolError::from)
    }
}

impl NumbersAnswer {
    pub fn new(id: u64, result: Result<u64, String>) -> Self {
        let (first, error) = match result {
            Ok(first) => (Some(first), None),
            Err(error) => (None, Some(error)),
        };
        Self { id, first, error }
    }

    /// The answer's outcome; an answer that carries both or neither is
    /// invalid.
    pub fn outcome(&self) -> Result<Result<u64, String>, ProtocolError> {
        garde::Validate::validate(self).map_err(ProtocolError::from)?;
        match (self.first, &self.error) {
            (Some(first), None) => Ok(Ok(first)),
            (None, Some(error)) => Ok(Err(error.clone())),
            _ => Err(ProtocolError::Invalid(
                "a numbers answer carries either its first number or its error".into(),
            )),
        }
    }
}
