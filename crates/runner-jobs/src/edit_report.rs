//! Convert retained edit snapshots, and the pages the job's commands
//! presented, to the runner's completed-job report.

use demi_command_protocol::{EDIT_FILE_BYTES, PresentedPage};
use demi_command_sdk::edits::Recorder;
use demi_runner_process::file_diff::line_counts;
use demi_runner_protocol::wire;
use std::io::Read;

/// What a job's commands reported: the files they changed, whether that list
/// was cut, and the pages they presented.
#[derive(Default)]
pub struct JobReport {
    pub files: Vec<wire::JobFileChange>,
    pub files_truncated: bool,
    pub presented: Vec<PresentedPage>,
}

pub fn finish(recorder: Option<&Recorder>) -> JobReport {
    let Some(recorder) = recorder else {
        return JobReport::default();
    };
    let journal = match recorder.report() {
        Ok(journal) => journal,
        Err(error) => {
            tracing::warn!("edit report failed: {error}");
            return JobReport::default();
        }
    };
    let files = journal
        .files
        .into_iter()
        .map(|mut file| {
            let mut added = 0;
            let mut removed = 0;
            for edit in &mut file.edits {
                let Some(modified) = &edit.modified else {
                    continue;
                };
                let sides = (|| -> std::io::Result<_> {
                    let original = edit.original.as_ref().map(read_snapshot).transpose()?;
                    let modified = read_snapshot(modified)?;
                    Ok((original, modified))
                })();
                match sides {
                    Ok((original, modified)) => {
                        let counts = line_counts(original.as_deref(), Some(&modified));
                        added += counts.0;
                        removed += counts.1;
                    }
                    Err(error) => {
                        tracing::warn!("edit snapshot read failed: {error}");
                        edit.original = None;
                        edit.modified = None;
                    }
                }
            }
            wire::JobFileChange {
                path: file.path,
                kind: file.kind,
                edits: file.edits,
                added,
                removed,
            }
        })
        .collect();
    JobReport {
        files,
        files_truncated: journal.files_truncated,
        presented: journal.presented,
    }
}

fn read_snapshot(path: &String) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((EDIT_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > EDIT_FILE_BYTES || !demi_command_protocol::is_text(&bytes) {
        return Err(std::io::Error::other("invalid edit snapshot"));
    }
    Ok(bytes)
}
