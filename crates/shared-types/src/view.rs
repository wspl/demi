//! Tool views (`runtime.md` § Views): the bounded data a `tool_call` block
//! carries for the user, never replayed to the model, with a type fixed per
//! tool by `kind`; and the output views of a command they and the live shell
//! status are built from.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{BlobRef, CommandId, MAX_SAFE_INTEGER};

/// A tool call's view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ToolView {
    /// A shell tool's command.
    Shell(#[garde(dive)] ShellToolView),
    /// A `shell` call the repeat guard suppressed: the script and how many
    /// times in a row it was asked for.
    RepeatedShell {
        #[garde(skip)]
        script: String,
        #[garde(skip)]
        count: u32,
    },
}

/// A command's status and the end of its output, as the shell tools saw it.
/// Its characters are Unicode scalar values, counted from the end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellToolView {
    #[garde(skip)]
    pub status: ShellViewStatus,
    #[garde(skip)]
    pub command_id: CommandId,
    /// Present once the command exited.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "i32")]
    #[garde(skip)]
    pub exit_code: Option<i32>,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub running_ms: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub idle_ms: u64,
    /// The last 32,768 characters of the merged stdout and stderr.
    #[garde(dive)]
    pub chunks: Vec<OutputChunk>,
    /// True when that window or the output itself was cut.
    #[garde(skip)]
    pub view_truncated: bool,
    /// The files the command changed, once it exited and changed some.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Vec<EditedFile>")]
    #[garde(dive)]
    pub files: Option<Vec<EditedFile>>,
    /// Present with `files`: whether the list was cut.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub files_truncated: Option<bool>,
    /// The renames and removals the command's embedded utilities made, in
    /// order, by which a request's file list follows its files to where
    /// they stand (`edit-tracking.md` § A request).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(dive)]
    pub path_changes: Vec<PathChange>,
    /// The files of the call's run up to it (`runtime.md` § Work groups),
    /// counted between before the run and after this call, which the
    /// backend computes as it stores the block; a page's light form keeps
    /// it only on the latest call of each run.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Vec<ChangedFile>")]
    #[garde(dive)]
    pub run_files: Option<Vec<ChangedFile>>,
    /// The files of the call's request up to it (`edit-tracking.md` § A
    /// request), with All Changes' counts and their edits; a page's light
    /// form keeps it only on the latest call of each request.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Vec<ChangedFile>")]
    #[garde(dive)]
    pub request_files: Option<Vec<ChangedFile>>,
}

/// Where a command is: running, exited, or stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShellViewStatus {
    Running,
    Exited,
    Aborted,
}

serde_plain::derive_display_from_serialize!(ShellViewStatus);
serde_plain::derive_fromstr_from_deserialize!(ShellViewStatus);

/// A run of a command's output from one stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputChunk {
    #[garde(skip)]
    pub stream: StreamKind,
    #[garde(skip)]
    pub text: String,
}

/// A command's output stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    Stdout,
    Stderr,
}

serde_plain::derive_display_from_serialize!(StreamKind);
serde_plain::derive_fromstr_from_deserialize!(StreamKind);

/// A file a command changed (`edit-tracking.md` § Edit copies): its line
/// counts and, per edit segment, the blobs of its two sides when they were
/// stored. One the command removed is `deleted`, and one it renamed
/// `renamed` with its old path, without edits (`edit-tracking.md` § The
/// report).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditedFile {
    /// Absolute, as the Host names it.
    #[garde(length(chars, min = 1))]
    pub path: String,
    #[garde(skip)]
    pub kind: EditKind,
    /// The path before a rename, with `renamed`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub from: Option<String>,
    #[garde(skip)]
    pub added: u32,
    #[garde(skip)]
    pub removed: u32,
    /// Whether `path` is in one of its Host's temporary directories
    /// (`edit-tracking.md` § Temporary files). Absent means false, as in
    /// every view stored before the field.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[garde(skip)]
    pub temporary: bool,
    /// None for a file the command removed or only renamed.
    #[garde(dive)]
    pub edits: Vec<EditSegment>,
}

/// How a file changed: created, changed where it was, removed, or moved
/// from another path, the kinds the working tree uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EditKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// A file of a run's or a request's list (`edit-tracking.md` § What the
/// conversation shows), as it stands after the call that stores the list:
/// its kind, the lines added and removed between its two ends, and its
/// edits in the order they were made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangedFile {
    /// Absolute, as the Host names it.
    #[garde(length(chars, min = 1))]
    pub path: String,
    #[garde(skip)]
    pub kind: EditKind,
    /// The path before a rename, with `renamed`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub from: Option<String>,
    /// The lines between its two ends; absent when they were not kept.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "LineCounts")]
    #[garde(dive)]
    pub lines: Option<LineCounts>,
    /// None for a file removed or renamed without an edit.
    #[garde(dive)]
    pub edits: Vec<FileEdit>,
}

/// Lines a change added and removed.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LineCounts {
    #[garde(skip)]
    pub added: u32,
    #[garde(skip)]
    pub removed: u32,
}

/// One edit of a listed file: one segment of one call's file, which the
/// Change view steps through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileEdit {
    /// The command of the call that made it.
    #[garde(skip)]
    pub call: CommandId,
    /// The file's path in that call, which a later call may have renamed.
    #[garde(length(chars, min = 1))]
    pub path: String,
    /// Which of the call's segments of the file it is.
    #[garde(skip)]
    pub segment: u32,
    /// The segment created the file, so nothing came before it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[garde(skip)]
    pub created: bool,
    /// The file's two sides; absent when they were not stored.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "EditCopies")]
    #[garde(skip)]
    pub copies: Option<EditCopies>,
}

serde_plain::derive_display_from_serialize!(EditKind);
serde_plain::derive_fromstr_from_deserialize!(EditKind);

/// A rename an embedded utility made, or a path an embedded `rm` removed: a
/// folder once, by its own path. Paths are absolute, as the Host names
/// them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PathChange {
    Renamed {
        #[garde(length(chars, min = 1))]
        from: String,
        #[garde(length(chars, min = 1))]
        to: String,
        /// Whether `to` is in one of its Host's temporary directories;
        /// absent means false.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[garde(skip)]
        temporary: bool,
    },
    Removed {
        #[garde(length(chars, min = 1))]
        path: String,
    },
}

/// One edit segment of a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditSegment {
    /// The file's two sides; absent when they were not stored.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "EditCopies")]
    #[garde(skip)]
    pub copies: Option<EditCopies>,
}

/// The two sides of an edit segment, blobs of the conversation owner's
/// namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditCopies {
    /// The file before the segment; the empty blob when the segment created
    /// it.
    #[garde(skip)]
    pub original: BlobRef,
    /// The file after the segment.
    #[garde(skip)]
    pub modified: BlobRef,
}

/// One output stream of a command since the last look.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct StreamView {
    /// Where the next look starts, in bytes: just after `delta`.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub offset: u64,
    /// The text since the last look.
    #[garde(skip)]
    pub delta: String,
    /// The end of the stream.
    #[garde(skip)]
    pub tail: String,
    /// The stream's length so far, in bytes.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub bytes: u64,
    #[garde(skip)]
    pub truncated: bool,
}

/// A command's merged stdout and stderr since the last look.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct OutputView {
    /// Where the next model look starts, in bytes: after the whole lines in
    /// `text`, or at the start of its unfinished last line while it runs.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub offset: u64,
    /// The line of the merged output that `text` starts in, from 1.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub line: u64,
    /// The merged text since the last look, repeating an unfinished line.
    #[garde(skip)]
    pub text: String,
    #[garde(skip)]
    pub tail: String,
    #[garde(dive)]
    pub chunks: Vec<OutputChunk>,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub bytes: u64,
    #[garde(skip)]
    pub truncated: bool,
}

/// A command's final stdout that was not text, described by its size: its
/// bytes never travel in a frame.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate,
)]
#[serde(rename_all = "camelCase")]
pub struct BinaryStdout {
    /// True when the stream exceeded `limitBytes` and was cut.
    #[garde(skip)]
    pub truncated: bool,
    /// The stream's whole length.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub total_bytes: u64,
    /// The ceiling that applied.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub limit_bytes: u64,
}
