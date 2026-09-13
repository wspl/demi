//! Convert retained edit snapshots to the runner's completed-job report.

use crate::{connection::wire, file_diff::line_counts};
use demi_command_service::edits::Recorder;
use demi_command_service::protocol::EDIT_FILE_BYTES;
use std::io::Read;

pub fn finish(recorder: Option<&Recorder>) -> (Vec<wire::JobExitFilesItem>, bool) {
    let Some(recorder) = recorder else {
        return (Vec::new(), false);
    };
    let journal = match recorder.report() {
        Ok(journal) => journal,
        Err(error) => {
            eprintln!("edit report failed: {error}");
            return (Vec::new(), false);
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
                        eprintln!("edit snapshot read failed: {error}");
                        edit.original = None;
                        edit.modified = None;
                    }
                }
            }
            wire::JobExitFilesItem {
                path: file.path,
                kind: file.kind,
                edits: file.edits,
                added,
                removed,
            }
        })
        .collect();
    (files, journal.files_truncated)
}

fn read_snapshot(path: &String) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((EDIT_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > EDIT_FILE_BYTES || bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
        return Err(std::io::Error::other("invalid edit snapshot"));
    }
    Ok(bytes)
}
