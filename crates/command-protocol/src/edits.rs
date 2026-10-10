//! The runner-owned file interface through which a native command reports
//! the files it edited (`edit-tracking.md`).

use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use super::invocation::without_nul;
use super::media::is_binary;

/// The most bytes of one file a job's edit record copies.
pub const EDIT_FILE_BYTES: usize = 8 * 1024 * 1024;
/// The most bytes a job's edit record copies in total.
pub const EDIT_JOB_BYTES: u64 = 64 * 1024 * 1024;
/// The most files a job's edit record lists.
pub const EDIT_JOB_FILES: usize = 500;
/// The most edit segments a job's edit record holds.
pub const EDIT_JOB_SEGMENTS: u64 = 1000;
/// The most renames and removals a job's edit record lists.
pub const EDIT_JOB_PATH_CHANGES: usize = 500;

/// Where an invoked command records its edits: the job's edit directory and
/// the lock that serializes writers to it. Both paths are absolute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct EditContext {
    #[garde(custom(absolute_path))]
    pub directory: String,
    #[garde(custom(absolute_path))]
    pub lock: String,
}

impl EditContext {
    pub fn validate(&self) -> Result<(), String> {
        garde::Validate::validate(self).map_err(|report| report.to_string())
    }
}

/// The copies of one edit segment: the file before and after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct EditCopies {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(length(min = 1), custom(without_nul)))]
    pub original: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(length(min = 1), custom(without_nul)))]
    pub modified: Option<String>,
}

/// Whether an edited file existed before the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditKind {
    Added,
    Modified,
}

/// One edited file and its segments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct EditFile {
    #[garde(length(min = 1), custom(without_nul))]
    pub path: String,
    #[garde(skip)]
    pub kind: EditKind,
    #[garde(length(max = EDIT_JOB_SEGMENTS as usize), dive)]
    pub edits: Vec<EditCopies>,
}

/// A rename an embedded utility made or a path an embedded `rm` removed,
/// which a request's file list follows across its calls
/// (`edit-tracking.md` § A request). Paths are absolute; a folder is named
/// once by its own path, never by each file in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum PathChange {
    Renamed {
        #[garde(length(min = 1), custom(without_nul))]
        from: String,
        #[garde(length(min = 1), custom(without_nul))]
        to: String,
    },
    Removed {
        #[garde(length(min = 1), custom(without_nul))]
        path: String,
    },
}

/// A job's edit record as the command left it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditJournal {
    #[garde(length(max = EDIT_JOB_FILES), dive)]
    pub files: Vec<EditFile>,
    /// The job's renames and removals, in the order they happened.
    #[garde(length(max = EDIT_JOB_PATH_CHANGES), dive)]
    pub path_changes: Vec<PathChange>,
    #[garde(range(max = EDIT_JOB_BYTES))]
    pub bytes_copied: u64,
    #[garde(range(max = EDIT_JOB_SEGMENTS))]
    pub next_segment: u64,
    #[garde(skip)]
    pub files_truncated: bool,
}

impl EditJournal {
    pub fn validate(&self) -> Result<(), String> {
        garde::Validate::validate(self).map_err(|report| report.to_string())
    }
}

fn absolute_path(value: &str, context: &()) -> garde::Result {
    without_nul(value, context)?;
    if !std::path::Path::new(value).is_absolute() {
        return Err(garde::Error::new("is not an absolute path"));
    }
    Ok(())
}

/// Why a file is not shown as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TextRefusal {
    #[error("The file is too large to show")]
    TooLarge,
    #[error("The file is binary")]
    NotText,
}

/// A file's bytes as the text the product shows: bytes that are not binary
/// ([`is_binary`], the rule edit tracking and line counts read by), with
/// each byte that is not UTF-8 as U+FFFD, up to the size an edit snapshot
/// keeps (`web-api.md` § File text and working tree changes). The backend's
/// file route and a runner's direct `text` channel both read a file's text
/// through it.
pub fn text_of(bytes: Vec<u8>) -> Result<String, TextRefusal> {
    if bytes.len() > EDIT_FILE_BYTES {
        return Err(TextRefusal::TooLarge);
    }
    if is_binary(&bytes) {
        return Err(TextRefusal::NotText);
    }
    Ok(String::from_utf8(bytes)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned()))
}
