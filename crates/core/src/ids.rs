//! The identities blocks and frames name. Whoever creates a thing chooses its
//! identity, such as the browser for a message it sends or the session for a
//! block it writes; an identity is any nonempty string, compared exactly.

use std::{borrow::Borrow, borrow::Cow, fmt, str::FromStr};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize, Serializer};

/// Why a string is not an identity.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("an identity must not be empty")]
pub struct EmptyId;

/// Declares an identity: a nonempty string that serializes as itself.
macro_rules! id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = EmptyId;

            fn try_from(value: String) -> Result<Self, EmptyId> {
                if value.is_empty() {
                    return Err(EmptyId);
                }
                Ok(Self(value))
            }
        }

        impl TryFrom<&str> for $name {
            type Error = EmptyId;

            fn try_from(value: &str) -> Result<Self, EmptyId> {
                Self::try_from(value.to_owned())
            }
        }

        impl FromStr for $name {
            type Err = EmptyId;

            fn from_str(value: &str) -> Result<Self, EmptyId> {
                Self::try_from(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl JsonSchema for $name {
            fn inline_schema() -> bool {
                true
            }

            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "string", "minLength": 1 })
            }
        }
    };
}

id!(
    /// A transcript block. A steer's id and an agent message's id are the ids
    /// of the blocks they become.
    BlockId
);
id!(
    /// A turn: every block a turn writes carries it. A message's id, which the
    /// browser chooses, is the id of the turn the message starts.
    TurnId
);
id!(
    /// An agent node: the root of a conversation or one of its subagents.
    NodeId
);
id!(
    /// A scheduled yield wakeup.
    WakeupId
);
id!(
    /// A shell of a node's shell environment.
    ShellId
);
id!(
    /// A command a shell runs; its handle for `shell_status`, `shell_write`
    /// and `shell_abort`.
    CommandId
);
id!(
    /// A message edit, which the browser chooses so that a repeated request
    /// is recognized.
    OperationId
);
