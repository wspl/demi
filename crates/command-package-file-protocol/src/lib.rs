//! The `demi.file` package's contract (`crates-and-packages.md`
//! § command-package-file-protocol): the arguments of its `file.*` operations. Paths are
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

/// `file.edit` as the command line gives it: SEARCH/REPLACE blocks on
/// stdin, or `--old` and `--new` for a one-line change. It decodes into
/// [`Edit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    /// Target file path
    #[garde(skip)]
    pub path: String,
    /// SEARCH/REPLACE blocks
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub blocks: Option<String>,
    /// Exact text to replace
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(length(min = 1))]
    pub old: Option<String>,
    /// Replacement text
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub new: Option<String>,
    /// 1-based occurrence to replace
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "usize")]
    #[garde(range(min = 1))]
    pub occurrence: Option<usize>,
    /// Line number used to choose the nearest occurrence
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "usize")]
    #[garde(range(min = 1))]
    pub context: Option<usize>,
}

/// `file.edit`, decoded: the file and the change made to it
/// (`commands.md` § File commands).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, garde::Validate)]
#[serde(try_from = "EditArgs")]
pub struct Edit {
    #[garde(skip)]
    pub path: String,
    #[garde(skip)]
    pub change: Change,
}

/// What an edit replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Whole lines, each block's SEARCH found exactly once in the file as
    /// it was; the blocks apply together.
    Blocks(Vec<Block>),
    /// Exact text anywhere in the file: its only match, the `occurrence`th,
    /// or the one nearest the line `context`.
    Text {
        old: String,
        new: String,
        occurrence: Option<usize>,
        context: Option<usize>,
    },
}

/// One SEARCH/REPLACE block: its lines, without their line endings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub search: Vec<String>,
    pub replace: Vec<String>,
}

/// The marker line that opens a block's SEARCH.
pub const SEARCH_MARKER: &str = "<<<<<<< SEARCH";
/// The marker line between a block's SEARCH and its REPLACE.
pub const DIVIDER_MARKER: &str = "=======";
/// The marker line that closes a block.
pub const REPLACE_MARKER: &str = ">>>>>>> REPLACE";

/// Why an edit's arguments are refused; its message is what the agent reads.
#[derive(Debug, thiserror::Error)]
pub enum EditArgsError {
    #[error(transparent)]
    Invalid(#[from] garde::Report),
    #[error(
        "Give the edit either as SEARCH/REPLACE blocks on stdin or as --old and --new, not both"
    )]
    BlocksAndText,
    #[error("--occurrence and --context choose among the matches of --old; blocks take neither")]
    BlocksAndChoice,
    #[error("Give SEARCH/REPLACE blocks on stdin, or --old and --new")]
    Nothing,
    #[error("--old needs --new")]
    OldWithoutNew,
    #[error("--new needs --old")]
    NewWithoutOld,
    #[error(
        "Line {line} of stdin is outside a SEARCH/REPLACE block; a block starts with the line {SEARCH_MARKER}"
    )]
    OutsideBlock { line: usize },
    #[error("Line {line} of stdin, {marker}, is out of place in block {block}")]
    MarkerOutOfPlace {
        line: usize,
        marker: &'static str,
        block: usize,
    },
    #[error("Block {block} ends before its {REPLACE_MARKER} line")]
    Unclosed { block: usize },
    #[error("Block {block} has an empty SEARCH; it must name the lines it replaces")]
    EmptySearch { block: usize },
}

impl TryFrom<EditArgs> for Edit {
    type Error = EditArgsError;

    fn try_from(args: EditArgs) -> Result<Self, EditArgsError> {
        garde::Validate::validate(&args)?;
        let blocks = args.blocks.as_deref().map(parse_blocks).transpose()?;
        let change = match (blocks.filter(|blocks| !blocks.is_empty()), args.old, args.new) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                return Err(EditArgsError::BlocksAndText);
            }
            (Some(_), None, None) if args.occurrence.is_some() || args.context.is_some() => {
                return Err(EditArgsError::BlocksAndChoice);
            }
            (Some(blocks), None, None) => Change::Blocks(blocks),
            (None, Some(old), Some(new)) => Change::Text {
                old,
                new,
                occurrence: args.occurrence,
                context: args.context,
            },
            (None, Some(_), None) => return Err(EditArgsError::OldWithoutNew),
            (None, None, Some(_)) => return Err(EditArgsError::NewWithoutOld),
            (None, None, None) => return Err(EditArgsError::Nothing),
        };
        Ok(Self {
            path: args.path,
            change,
        })
    }
}

/// The SEARCH/REPLACE blocks of `text`, whose markers are whole lines;
/// blank lines may stand between blocks, and nothing else.
fn parse_blocks(text: &str) -> Result<Vec<Block>, EditArgsError> {
    enum Part {
        Outside,
        Search(Vec<String>),
        Replace(Vec<String>, Vec<String>),
    }
    let mut blocks = Vec::new();
    let mut part = Part::Outside;
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let block = blocks.len() + 1;
        let marker = [SEARCH_MARKER, DIVIDER_MARKER, REPLACE_MARKER]
            .into_iter()
            .find(|marker| *marker == line);
        part = match (part, marker) {
            (Part::Outside, Some(SEARCH_MARKER)) => Part::Search(Vec::new()),
            (Part::Outside, None) if line.trim().is_empty() => Part::Outside,
            (Part::Outside, _) => return Err(EditArgsError::OutsideBlock { line: number }),
            (Part::Search(search), Some(DIVIDER_MARKER)) => {
                if search.is_empty() {
                    return Err(EditArgsError::EmptySearch { block });
                }
                Part::Replace(search, Vec::new())
            }
            (Part::Replace(search, replace), Some(REPLACE_MARKER)) => {
                blocks.push(Block { search, replace });
                Part::Outside
            }
            (Part::Search(_) | Part::Replace(..), Some(marker)) => {
                return Err(EditArgsError::MarkerOutOfPlace {
                    line: number,
                    marker,
                    block,
                });
            }
            (Part::Search(mut search), None) => {
                search.push(line.to_owned());
                Part::Search(search)
            }
            (Part::Replace(search, mut replace), None) => {
                replace.push(line.to_owned());
                Part::Replace(search, replace)
            }
        };
    }
    if !matches!(part, Part::Outside) {
        return Err(EditArgsError::Unclosed {
            block: blocks.len() + 1,
        });
    }
    Ok(blocks)
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
    "file.edit" => Edit(Edit),
    "file.patch" => Patch(PatchArgs),
}
