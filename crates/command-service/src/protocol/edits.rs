//! The runner-owned file interface through which a native command reports
//! the files it edited (`edit-tracking.md`).

use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use super::invocation::without_nul;

/// The most bytes of one file a job's edit record copies.
pub const EDIT_FILE_BYTES: usize = 8 * 1024 * 1024;
/// The most bytes a job's edit record copies in total.
pub const EDIT_JOB_BYTES: u64 = 64 * 1024 * 1024;
/// The most files a job's edit record lists.
pub const EDIT_JOB_FILES: usize = 500;
/// The most edit segments a job's edit record holds.
pub const EDIT_JOB_SEGMENTS: u64 = 1000;

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
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[garde(inner(length(min = 1), custom(without_nul)))]
    pub original: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
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

/// A job's edit record as the command left it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditJournal {
    #[garde(length(max = EDIT_JOB_FILES), dive)]
    pub files: Vec<EditFile>,
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
