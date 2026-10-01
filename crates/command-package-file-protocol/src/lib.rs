//! The `demi.file` package's contract (`crates-and-packages.md`
//! § file-protocol): the arguments of its `file.*` operations. Paths are
//! relative to the invocation's working directory; the handler resolves and
//! checks them. It holds types and their checks only; the operations live in
//! `demi-file`.

use demi_shared_types::DecodeError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

/// The package's id, which its release descriptor names and the coding
/// agent's commands bind to.
pub const PACKAGE: &str = "demi.file";

/// Why an invocation could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    /// The name is not one of the package's operations.
    #[error("unknown operation {0}")]
    Unknown(String),
    /// The operation's arguments are refused.
    #[error(transparent)]
    Invalid(#[from] DecodeError),
}

/// `file.read`: writes the file's bytes to stdout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ReadArgs {
    /// File path to read
    #[garde(skip)]
    pub path: String,
}

/// `file.create`: creates a new file; an existing file is left as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct CreateArgs {
    /// Target file path
    #[garde(skip)]
    pub path: String,
    /// File content
    #[garde(skip)]
    pub content: String,
}

/// `file.edit`: replaces one occurrence of exact text in an existing file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    /// Target file path
    #[garde(skip)]
    pub path: String,
    /// Exact text to replace
    #[garde(length(min = 1))]
    pub old: String,
    /// Replacement text
    #[garde(skip)]
    pub new: String,
    /// 1-based occurrence to replace
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "usize")]
    #[garde(range(min = 1))]
    pub occurrence: Option<usize>,
    /// Line number used to choose the nearest occurrence
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "usize")]
    #[garde(range(min = 1))]
    pub context: Option<usize>,
}

/// `file.patch`: applies a unified diff to one or more files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PatchArgs {
    /// Unified diff content
    #[garde(skip)]
    pub patch: String,
}

/// Declares the operations: the enum of decoded arguments and the operation
/// list.
macro_rules! operations {
    ($($name:literal => $variant:ident($args:ty),)*) => {
        /// A decoded invocation: the operation and its checked arguments.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum Operation {
            $($variant($args),)*
        }

        impl Operation {
            /// Decodes the arguments of the operation named `operation`.
            pub fn parse(operation: &str, args: serde_json::Value) -> Result<Self, OperationError> {
                match operation {
                    $($name => Ok(demi_shared_types::decode_value(args).map(Self::$variant)?),)*
                    _ => Err(OperationError::Unknown(operation.to_owned())),
                }
            }
        }

        /// The package's operations, as its descriptor lists them.
        pub const OPERATIONS: &[&str] = &[$($name,)*];
    };
}

operations! {
    "file.read" => Read(ReadArgs),
    "file.create" => Create(CreateArgs),
    "file.edit" => Edit(EditArgs),
    "file.patch" => Patch(PatchArgs),
}
