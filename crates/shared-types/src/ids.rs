//! The identities blocks and frames name. Whoever creates a thing chooses its
//! identity, such as the web app for a message it sends or the session for a
//! block it writes; an identity is any nonempty string, compared exactly.
//! Other crates declare their identities with [`id!`](crate::id).

/// Why a string is not an identity.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("an identity must not be empty")]
pub struct EmptyId;

/// Declares an identity: a string newtype that serializes as the string,
/// compares exactly, and holds only the strings its check accepts, since
/// every way of making one, decoding included, runs the check.
///
/// `id!(Name)` accepts any nonempty string and refuses the empty one with
/// [`EmptyId`]. `id!(Name, check = f, error = E, schema = { .. })` accepts
/// the strings for which `f(&str) -> Result<(), E>` answers `Ok`, and
/// describes them to JSON Schema, and so to the web app, with the given
/// schema object.
///
/// ```ignore
/// demi_shared_types::id!(
///     /// A user account.
///     UserId
/// );
/// ```
#[macro_export]
macro_rules! id {
    ($(#[$meta:meta])* $name:ident) => {
        $crate::id!(
            $(#[$meta])* $name,
            check = $crate::__private::nonempty,
            error = $crate::EmptyId,
            schema = { "type": "string", "minLength": 1 }
        );
    };
    ($(#[$meta:meta])* $name:ident, check = $check:path, error = $error:ty, schema = $schema:tt) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl ::core::convert::TryFrom<String> for $name {
            type Error = $error;

            fn try_from(value: String) -> ::core::result::Result<Self, $error> {
                $check(&value)?;
                Ok(Self(value))
            }
        }

        impl ::core::convert::TryFrom<&str> for $name {
            type Error = $error;

            fn try_from(value: &str) -> ::core::result::Result<Self, $error> {
                Self::try_from(value.to_owned())
            }
        }

        impl ::core::str::FromStr for $name {
            type Err = $error;

            fn from_str(value: &str) -> ::core::result::Result<Self, $error> {
                Self::try_from(value)
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl ::core::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl ::core::convert::AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl $crate::__private::serde::Serialize for $name {
            fn serialize<S: $crate::__private::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::core::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> $crate::__private::serde::Deserialize<'de> for $name {
            fn deserialize<D: $crate::__private::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::core::result::Result<Self, D::Error> {
                let value = <String as $crate::__private::serde::Deserialize>::deserialize(deserializer)?;
                Self::try_from(value).map_err(<D::Error as $crate::__private::serde::de::Error>::custom)
            }
        }

        impl $crate::__private::schemars::JsonSchema for $name {
            fn inline_schema() -> bool {
                true
            }

            fn schema_name() -> ::std::borrow::Cow<'static, str> {
                ::core::stringify!($name).into()
            }

            fn json_schema(
                _: &mut $crate::__private::schemars::SchemaGenerator,
            ) -> $crate::__private::schemars::Schema {
                $crate::__private::schemars::json_schema!($schema)
            }
        }
    };
}

/// What [`id!`](crate::id)'s expansion reaches in this crate, wherever it is
/// expanded.
#[doc(hidden)]
pub mod __private {
    pub use schemars;
    pub use serde;

    /// The check of an identity that may be any nonempty string.
    pub fn nonempty(value: &str) -> Result<(), super::EmptyId> {
        if value.is_empty() {
            return Err(super::EmptyId);
        }
        Ok(())
    }
}

crate::id!(
    /// A transcript block. A steer's id and an agent message's id are the ids
    /// of the blocks they become.
    BlockId
);
crate::id!(
    /// A turn: every block a turn writes carries it. A message's id, which the
    /// web app chooses, is the id of the turn the message starts.
    TurnId
);
crate::id!(
    /// An agent node: the root of a conversation or one of its subagents.
    NodeId
);
crate::id!(
    /// A scheduled yield wakeup.
    WakeupId
);
crate::id!(
    /// A shell of a node's shell environment.
    ShellId
);
crate::id!(
    /// A command a shell runs; its handle for `shell_status`, `shell_write`
    /// and `shell_abort`.
    CommandId
);
crate::id!(
    /// A message edit, which the web app chooses so that a repeated request
    /// is recognized.
    OperationId
);

/// A sequence of the numbers the model knows a conversation's things by
/// (`runtime.md` § Identifiers the model sees): each is given once, in order,
/// across crashes, restores and Forks.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Sequence {
    /// Commands, from 1: `17`.
    Command,
    /// Shells, from 1: `3`.
    Shell,
    /// Subagents, from 1; the root is agent 0.
    Agent,
    /// Conversation browser tabs, from 1: `t7`. The `demi.browser` service
    /// reserves them a few at a time (`native-runtime.md` § Conversation
    /// numbers).
    Tab,
    /// Attachments the agent uploaded, from 1: `a3`. `demi attachment
    /// upload` takes one per file it stores (`commands.md` § Attachment
    /// commands).
    Attachment,
}

serde_plain::derive_display_from_serialize!(Sequence);
serde_plain::derive_fromstr_from_deserialize!(Sequence);
