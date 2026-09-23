//! The arguments of the `file.*` operations. Paths are relative to the
//! invocation's working directory; the handler resolves and checks them.

use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::DecodeError;

/// The file operations, as the package descriptor lists them.
pub const OPERATIONS: &[&str] = &["file.read", "file.create", "file.edit", "file.patch"];

/// `file.read`: writes the file's bytes to stdout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ReadArgs {
    /// File path to read
    #[garde(skip)]
    pub path: String,
}

/// `file.create`: creates a new file; an existing file is left as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
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
    #[garde(range(min = 1))]
    pub occurrence: Option<usize>,
    /// Line number used to choose the nearest occurrence
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[garde(range(min = 1))]
    pub context: Option<usize>,
}

/// `file.patch`: applies a unified diff to one or more files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PatchArgs {
    /// Unified diff content
    #[garde(skip)]
    pub patch: String,
}

/// A decoded `file.*` invocation: the operation and its checked arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOperation {
    Read(ReadArgs),
    Create(CreateArgs),
    Edit(EditArgs),
    Patch(PatchArgs),
}

impl FileOperation {
    /// Decodes the arguments of the operation named `operation`.
    pub fn parse(operation: &str, args: serde_json::Value) -> Result<Self, DecodeError> {
        match operation {
            "file.read" => crate::decode(args).map(Self::Read),
            "file.create" => crate::decode(args).map(Self::Create),
            "file.edit" => crate::decode(args).map(Self::Edit),
            "file.patch" => crate::decode(args).map(Self::Patch),
            _ => Err(DecodeError::UnknownOperation(operation.to_owned())),
        }
    }
}
