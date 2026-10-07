//! The runner-owned file interface through which a native command reports
//! the files it edited (`edit-tracking.md`), and the pages it presented to
//! the user (`preview.md` § Presenting a page).

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
/// The most pages a job's commands present; a later one is left out.
pub const PRESENTED_JOB_PAGES: usize = 32;

/// A page a job's command presented with `demi browser present`: the card
/// the command's block shows (`preview.md` § Presenting a page).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PresentedPage {
    /// The tab of the agent's browser.
    #[garde(length(min = 1, max = 64))]
    pub tab: String,
    #[garde(length(max = 4096))]
    pub title: String,
    #[garde(length(min = 1, max = 4096))]
    pub url: String,
}

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
    /// The pages the job's commands presented, the latest of each tab.
    #[serde(default)]
    #[garde(length(max = PRESENTED_JOB_PAGES), dive)]
    pub presented: Vec<PresentedPage>,
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

/// Whether `bytes` are text, which edit tracking and line counts read:
/// UTF-8 without a NUL byte. Binary and non-UTF-8 content are treated alike
/// (`edit-tracking.md` § Scope).
pub fn is_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// Why a file is not shown as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TextRefusal {
    #[error("The file is too large to show")]
    TooLarge,
    #[error("The file is not UTF-8 text")]
    NotText,
}

/// A file's bytes as the text the product shows, which edit tracking and
/// line counts read as text: UTF-8 without a NUL byte, up to the size an
/// edit snapshot keeps (`web-api.md` § File text and working tree changes).
/// The backend's file route and a runner's direct `text` channel both read
/// a file's text through it.
pub fn text_of(bytes: Vec<u8>) -> Result<String, TextRefusal> {
    if bytes.len() > EDIT_FILE_BYTES {
        return Err(TextRefusal::TooLarge);
    }
    if !is_text(&bytes) {
        return Err(TextRefusal::NotText);
    }
    // `is_text` checked the encoding.
    Ok(String::from_utf8(bytes).expect("text is UTF-8"))
}
