//! Convert retained edit snapshots to the runner's completed-job report.

use demi_command_protocol::{EDIT_FILE_BYTES, PathChange};
use demi_command_sdk::edits::Recorder;
use demi_command_protocol::line_counts;
use demi_runner_protocol::wire;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// What a job's `job_exit` reports of its edits (`edit-tracking.md`
/// § The report).
#[derive(Default)]
pub struct EditReport {
    pub files: Vec<wire::JobFileChange>,
    pub path_changes: Vec<wire::JobPathChange>,
    pub truncated: bool,
}

pub fn finish(recorder: Option<&Recorder>) -> EditReport {
    let Some(recorder) = recorder else {
        return EditReport::default();
    };
    let journal = match recorder.report() {
        Ok(journal) => journal,
        Err(error) => {
            tracing::warn!("edit report failed: {error}");
            return EditReport::default();
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
                temporary: is_temporary(Path::new(&file.path)),
                path: file.path,
                kind: file.kind,
                edits: file.edits,
                added,
                removed,
            }
        })
        .collect();
    EditReport {
        files,
        path_changes: journal
            .path_changes
            .into_iter()
            .map(|change| match change {
                PathChange::Renamed { from, to } => wire::JobPathChange::Renamed {
                    temporary: is_temporary(Path::new(&to)),
                    from,
                    to,
                },
                PathChange::Removed { path } => wire::JobPathChange::Removed { path },
            })
            .collect(),
        truncated: journal.files_truncated,
    }
}

fn read_snapshot(path: &String) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((EDIT_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > EDIT_FILE_BYTES || demi_command_protocol::is_binary(&bytes) {
        return Err(std::io::Error::other("invalid edit snapshot"));
    }
    Ok(bytes)
}

/// Whether `path` is in one of the Host's temporary directories
/// (`edit-tracking.md` § Temporary files): the directories as named and as
/// their links resolve, compared component by component, so `/tmp/x` and
/// macOS's `/private/tmp/x` both are and `/tmpfoo/x` is not.
fn is_temporary(path: &Path) -> bool {
    let resolved = resolved(path);
    temporary_directories()
        .iter()
        .any(|directory| path.starts_with(directory) || resolved.starts_with(directory))
        || in_per_user_temporary(&resolved)
}

/// The temporary directories every Host names: the process's own
/// (`$TMPDIR`, or `%TMP%` on Windows), `/tmp` and `/var/tmp`, and
/// `%TEMP%`, each as named and as resolved. Read once: the runner's
/// environment does not change while it runs.
fn temporary_directories() -> &'static [PathBuf] {
    static DIRECTORIES: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRECTORIES.get_or_init(|| {
        let mut named = vec![std::env::temp_dir()];
        if cfg!(unix) {
            named.extend([PathBuf::from("/tmp"), PathBuf::from("/var/tmp")]);
        }
        if cfg!(windows) {
            named.extend(["TEMP", "TMP"].into_iter().filter_map(std::env::var_os).map(PathBuf::from));
        }
        let mut directories = Vec::new();
        for directory in named {
            // A directory that does not resolve, such as a missing
            // `/var/tmp`, still names where its files would be.
            if let Ok(canonical) = std::fs::canonicalize(&directory) {
                directories.push(canonical);
            }
            directories.push(directory);
        }
        directories.sort();
        directories.dedup();
        directories
    })
}

/// Whether a resolved path is in macOS's per-user temporary directory,
/// `/private/var/folders/<x>/<y>/T`, whichever user's it is.
fn in_per_user_temporary(resolved: &Path) -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let Ok(rest) = resolved.strip_prefix("/private/var/folders") else {
        return false;
    };
    rest.components().nth(2).is_some_and(|part| part.as_os_str() == "T")
}

/// `path` with the links of its folders resolved, from the deepest one
/// that exists, and its own name kept: a link named `notes.md` that points
/// into `/tmp` is where its name is, not where it points. A path none of
/// whose folders resolve stays as it is.
fn resolved(path: &Path) -> PathBuf {
    let mut names = Vec::new();
    let mut folder = path;
    loop {
        let (Some(parent), Some(name)) = (folder.parent(), folder.file_name()) else {
            return path.to_owned();
        };
        names.push(name);
        folder = parent;
        if let Ok(mut resolved) = std::fs::canonicalize(folder) {
            resolved.extend(names.iter().rev());
            return resolved;
        }
    }
}
