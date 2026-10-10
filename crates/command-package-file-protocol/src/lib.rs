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
    /// `file.edit`'s arguments are refused; the handler reports a path the
    /// error names as it resolves it.
    #[error(transparent)]
    Edit(#[from] EditArgsError),
}

/// `file.view`: shows the model each file, or stdin, as a medium, in order
/// (`runtime.md` § What `demi file view` shows).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ViewArgs {
    /// Images, videos or PDFs to show, in order; - or none reads stdin
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Vec<String>")]
    #[garde(skip)]
    pub path: Option<Vec<String>>,
}

/// `file.edit` as the command line gives it: SEARCH/REPLACE blocks on
/// stdin, each file's after a line naming it or all of them the path
/// argument's, or `--old` and `--new` for a one-line change to the path
/// argument. It decodes into [`Edit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    /// The file to change, when stdin names none
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub path: Option<String>,
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

/// `file.edit`, decoded (`commands.md` § Editing files) from checked
/// [`EditArgs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// SEARCH/REPLACE blocks, each with the file it changes, in the order
    /// of stdin; the same file may come more than once.
    Blocks(Vec<FileBlocks>),
    /// Exact text anywhere in the file, at the match `choice` names.
    Text {
        path: String,
        old: String,
        new: String,
        choice: Choice,
    },
}

/// The blocks stdin gives for one file, after the line naming it or, with
/// no such line, for the path argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileBlocks {
    pub path: String,
    pub blocks: Vec<Block>,
}

/// Which match of `--old` an edit replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// The only one.
    Only,
    /// The nth, 1-based (`--occurrence`).
    Occurrence(usize),
    /// The one nearest this 1-based line (`--context`).
    Context(usize),
}

/// One SEARCH/REPLACE block. A SEARCH without sections stands for the text
/// of its lines, joined by line endings, found anywhere in the file, part
/// of a line included; a SEARCH with a section matches whole lines. An
/// empty SEARCH creates its file with the REPLACE as its content. A REPLACE
/// holds no section, or as many as the SEARCH, each standing for the lines
/// its SEARCH section matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub search: Vec<BlockLine>,
    pub replace: Vec<BlockLine>,
}

impl Block {
    /// Whether the block creates its file.
    pub fn creates(&self) -> bool {
        self.search.is_empty()
    }
}

/// A line of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockLine {
    /// A line of text, without its line ending.
    Text(String),
    /// [`SECTION_MARKER`]: in a SEARCH, any run of lines, none included, as
    /// short as the rest of the block allows; in a REPLACE, the lines the
    /// SEARCH's section of the same rank matched.
    Section,
}

/// The marker line that opens a block's SEARCH.
pub const SEARCH_MARKER: &str = "<<<<<<< SEARCH";
/// The marker line between a block's SEARCH and its REPLACE.
pub const DIVIDER_MARKER: &str = "=======";
/// The marker line that closes a block.
pub const REPLACE_MARKER: &str = ">>>>>>> REPLACE";
/// The line that stands for a section of the file, the markers' width.
pub const SECTION_MARKER: &str = ".......";

/// Why an edit's arguments are refused; its message is what the agent reads.
#[derive(Debug, thiserror::Error)]
pub enum EditArgsError {
    #[error(
        "Give the edit either as SEARCH/REPLACE blocks on stdin or as --old and --new, not both"
    )]
    BlocksAndText,
    #[error("--occurrence and --context choose among the matches of --old; blocks take neither")]
    BlocksAndChoice,
    #[error("Give SEARCH/REPLACE blocks on stdin, or --old and --new")]
    Nothing,
    #[error("Give --occurrence or --context, not both")]
    OccurrenceAndContext,
    #[error("--old needs --new")]
    OldWithoutNew,
    #[error("--new needs --old")]
    NewWithoutOld,
    #[error("--old and --new change the file the path argument names; give it")]
    TextWithoutPath,
    #[error(
        "Name the files either with the path argument or with path lines on stdin, not both: line {line} of stdin names {path}"
    )]
    PathAndPathLines { line: usize, path: String },
    #[error(
        "Block {block} has no file: put the file's path on a line before it, or pass it as the path argument"
    )]
    BlockWithoutFile { block: usize },
    #[error("Line {line} of stdin names {path}, but no block follows it")]
    PathWithoutBlocks { line: usize, path: String },
    #[error("Line {line} of stdin, {marker}, is out of place in block {block}")]
    MarkerOutOfPlace {
        line: usize,
        marker: &'static str,
        block: usize,
    },
    #[error("Block {block} ends before its {REPLACE_MARKER} line")]
    Unclosed { block: usize },
    #[error(
        "{path}: block {block}: its REPLACE has {replace} {SECTION_MARKER} line(s); it needs none, to replace the whole match, or {search}, one for each in its SEARCH"
    )]
    Sections {
        path: String,
        block: usize,
        search: usize,
        replace: usize,
    },
}

impl TryFrom<EditArgs> for Edit {
    type Error = EditArgsError;

    /// The edit `args` give; [`Operation::parse`] has checked their fields.
    fn try_from(args: EditArgs) -> Result<Self, EditArgsError> {
        let choice = match (args.occurrence, args.context) {
            (Some(_), Some(_)) => return Err(EditArgsError::OccurrenceAndContext),
            (Some(occurrence), None) => Some(Choice::Occurrence(occurrence)),
            (None, Some(context)) => Some(Choice::Context(context)),
            (None, None) => None,
        };
        let files = args
            .blocks
            .as_deref()
            .map(|text| parse_blocks(text, args.path.as_deref()))
            .transpose()?
            .filter(|files| !files.is_empty());
        match (files, args.old, args.new) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => Err(EditArgsError::BlocksAndText),
            (Some(_), None, None) if choice.is_some() => Err(EditArgsError::BlocksAndChoice),
            (Some(files), None, None) => Ok(Self::Blocks(files)),
            (None, Some(old), Some(new)) => Ok(Self::Text {
                path: args.path.ok_or(EditArgsError::TextWithoutPath)?,
                old,
                new,
                choice: choice.unwrap_or(Choice::Only),
            }),
            (None, Some(_), None) => Err(EditArgsError::OldWithoutNew),
            (None, None, Some(_)) => Err(EditArgsError::NewWithoutOld),
            (None, None, None) => Err(EditArgsError::Nothing),
        }
    }
}

/// Whether `line` is `marker`, which may carry trailing spaces or tabs.
fn is_marker(line: &str, marker: &str) -> bool {
    line.trim_end_matches([' ', '\t']) == marker
}

/// The SEARCH/REPLACE blocks of `text`, grouped by the file each changes:
/// the one the last path line before it names, or `path` when stdin names
/// none. Markers are whole lines; outside the blocks, a line names a file
/// and blank lines may stand anywhere.
fn parse_blocks(text: &str, path: Option<&str>) -> Result<Vec<FileBlocks>, EditArgsError> {
    enum Part {
        Outside,
        Search(Vec<BlockLine>),
        Replace(Vec<BlockLine>, Vec<BlockLine>),
    }
    // The path argument's file, which takes every block when no line names
    // another.
    let mut files: Vec<FileBlocks> = path
        .map(|path| FileBlocks {
            path: path.to_owned(),
            blocks: Vec::new(),
        })
        .into_iter()
        .collect();
    // The line that named the current file, while no block followed it.
    let mut named: Option<(usize, String)> = None;
    // How many blocks each file had, to number its blocks in messages.
    let mut counts = std::collections::HashMap::<String, usize>::new();
    let mut part = Part::Outside;
    let mut block = 0;
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let marker = [SEARCH_MARKER, DIVIDER_MARKER, REPLACE_MARKER]
            .into_iter()
            .find(|marker| is_marker(line, marker));
        let block_line = || {
            if is_marker(line, SECTION_MARKER) {
                BlockLine::Section
            } else {
                BlockLine::Text(line.to_owned())
            }
        };
        part = match (part, marker) {
            (Part::Outside, Some(SEARCH_MARKER)) => {
                block += 1;
                Part::Search(Vec::new())
            }
            (Part::Outside, None) if line.trim().is_empty() => Part::Outside,
            (Part::Outside, None) => {
                let file = line.trim().to_owned();
                if path.is_some() {
                    return Err(EditArgsError::PathAndPathLines { line: number, path: file });
                }
                if let Some((line, path)) = named.take() {
                    return Err(EditArgsError::PathWithoutBlocks { line, path });
                }
                files.push(FileBlocks {
                    path: file.clone(),
                    blocks: Vec::new(),
                });
                named = Some((number, file));
                Part::Outside
            }
            (Part::Search(search), Some(DIVIDER_MARKER)) => Part::Replace(search, Vec::new()),
            (Part::Replace(search, replace), Some(REPLACE_MARKER)) => {
                named = None;
                let Some(file) = files.last_mut() else {
                    return Err(EditArgsError::BlockWithoutFile { block });
                };
                let count = counts.entry(file.path.clone()).or_default();
                *count += 1;
                let sections = |lines: &[BlockLine]| {
                    lines
                        .iter()
                        .filter(|line| **line == BlockLine::Section)
                        .count()
                };
                let (in_search, in_replace) = (sections(&search), sections(&replace));
                if in_replace != 0 && in_replace != in_search {
                    return Err(EditArgsError::Sections {
                        path: file.path.clone(),
                        block: *count,
                        search: in_search,
                        replace: in_replace,
                    });
                }
                file.blocks.push(Block { search, replace });
                Part::Outside
            }
            (Part::Search(_) | Part::Replace(..), Some(marker)) | (Part::Outside, Some(marker)) => {
                return Err(EditArgsError::MarkerOutOfPlace {
                    line: number,
                    marker,
                    block: block.max(1),
                });
            }
            (Part::Search(mut search), None) => {
                search.push(block_line());
                Part::Search(search)
            }
            (Part::Replace(search, mut replace), None) => {
                replace.push(block_line());
                Part::Replace(search, replace)
            }
        };
    }
    if !matches!(part, Part::Outside) {
        return Err(EditArgsError::Unclosed { block });
    }
    if let Some((line, path)) = named {
        return Err(EditArgsError::PathWithoutBlocks { line, path });
    }
    // Only the path argument's file can be left without blocks: stdin had
    // none.
    files.retain(|file| !file.blocks.is_empty());
    Ok(files)
}

/// `file.patch`: applies a unified diff to one or more files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PatchArgs {
    /// Unified diff content
    #[garde(skip)]
    pub patch: String,
}

/// Decodes and checks an operation's arguments.
fn decode<T>(args: serde_json::Value) -> Result<T, OperationError>
where
    T: serde::de::DeserializeOwned + garde::Validate<Context = ()>,
{
    Ok(demi_shared_types::decode_value(args)?)
}

/// Decodes `file.edit`'s arguments: checked [`EditArgs`], then the edit
/// they give.
fn decode_edit(args: serde_json::Value) -> Result<Edit, OperationError> {
    Ok(Edit::try_from(decode::<EditArgs>(args)?)?)
}

/// Declares the operations: the enum of decoded arguments, each decoded by
/// its function, and the operation list.
macro_rules! operations {
    ($($name:literal => $variant:ident($args:ty) by $decode:ident,)*) => {
        /// A decoded invocation: the operation and its checked arguments.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum Operation {
            $($variant($args),)*
        }

        impl Operation {
            /// Decodes the arguments of the operation named `operation`.
            pub fn parse(operation: &str, args: serde_json::Value) -> Result<Self, OperationError> {
                match operation {
                    $($name => $decode(args).map(Self::$variant),)*
                    _ => Err(OperationError::Unknown(operation.to_owned())),
                }
            }
        }

        /// The package's operations, as its descriptor lists them.
        pub const OPERATIONS: &[&str] = &[$($name,)*];
    };
}

operations! {
    "file.view" => View(ViewArgs) by decode,
    "file.edit" => Edit(Edit) by decode_edit,
    "file.patch" => Patch(PatchArgs) by decode,
}
