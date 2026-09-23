//! Identifiers of the records the browser names.

use std::borrow::Cow;

use demi_core::EmptyId;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

/// A user account's id, which the backend assigns: like every identity, a
/// nonempty string compared exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UserId(String);

impl UserId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for UserId {
    type Error = EmptyId;

    fn try_from(id: String) -> Result<Self, EmptyId> {
        if id.is_empty() {
            return Err(EmptyId);
        }
        Ok(Self(id))
    }
}

impl From<UserId> for String {
    fn from(id: UserId) -> Self {
        id.0
    }
}

impl JsonSchema for UserId {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "UserId".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "minLength": 1 })
    }
}
