//! Convert retained edit snapshots to the runner's completed-job report.

use demi_command_protocol::{EDIT_FILE_BYTES, PathChange};
use demi_command_sdk::edits::Recorder;
use demi_command_protocol::line_counts;
use demi_runner_protocol::wire;
use crate::scratch::Scratch;
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

/// The report of what `recorder` recorded of a job that started in
/// `working`.
pub fn finish(recorder: Option<&Recorder>, scratch: &Scratch, working: &Path) -> EditReport {
    let Some(recorder) = recorder else {
        return EditReport::default();
    };
    let working = Directory::new(working.to_owned());
    let is_temporary = |path: &str| is_temporary(Path::new(path), scratch, &working);
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
                temporary: is_temporary(&file.path),
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
                    temporary: is_temporary(&to),
                    from,
                    to,
                },
                PathChange::Removed { path } => wire::JobPathChange::Removed {
                    temporary: is_temporary(&path),
                    path,
                },
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

/// Whether `path` is temporary (`edit-tracking.md` § Temporary files): in
/// a conversation's scratch directory, or else outside the job's `working`
/// directory and in one of the Host's temporary directories. The scratch
/// directory comes first: the working directory, such as the user's home,
/// may hold it.
fn is_temporary(path: &Path, scratch: &Scratch, working: &Directory) -> bool {
    let resolved = resolved(path);
    if scratch.directory().holds(path, &resolved) {
        return true;
    }
    if working.holds(path, &resolved) {
        return false;
    }
    temporary_directories()
        .iter()
        .any(|directory| directory.holds(path, &resolved))
        || in_per_user_temporary(&resolved)
}

/// A directory as named and as its links resolve. It holds a path when
/// the path, as named or with its folders' links resolved, is in either,
/// compared component by component, so `/tmp` holds `/tmp/x` and macOS's
/// `/private/tmp/x` and not `/tmpfoo/x`.
#[derive(Clone, Debug)]
pub(crate) struct Directory {
    named: PathBuf,
    resolved: Option<PathBuf>,
}

impl Directory {
    /// `named` and where it resolves; one that does not resolve, such as a
    /// missing `/var/tmp`, still names where its files would be.
    pub(crate) fn new(named: PathBuf) -> Self {
        let resolved = std::fs::canonicalize(&named).ok();
        Self { named, resolved }
    }

    /// `name` in `parent`, as named and as `parent` resolves, for a
    /// directory that may not exist yet.
    pub(crate) fn within(parent: &Path, name: &str) -> Self {
        Self {
            named: parent.join(name),
            resolved: std::fs::canonicalize(parent).ok().map(|parent| parent.join(name)),
        }
    }

    /// The directory as named.
    pub(crate) fn path(&self) -> &Path {
        &self.named
    }

    /// Whether it holds `path`, whose folders' links resolve to `resolved`.
    fn holds(&self, path: &Path, resolved: &Path) -> bool {
        std::iter::once(&self.named)
            .chain(&self.resolved)
            .any(|directory| path.starts_with(directory) || resolved.starts_with(directory))
    }
}

/// The temporary directories every Host names: the process's own
/// (`$TMPDIR`, or `%TMP%` on Windows), `/tmp` and `/var/tmp`, and
/// `%TEMP%`. Read once: the runner's environment does not change while it
/// runs.
fn temporary_directories() -> &'static [Directory] {
    static DIRECTORIES: OnceLock<Vec<Directory>> = OnceLock::new();
    DIRECTORIES.get_or_init(|| {
        let mut named = vec![std::env::temp_dir()];
        if cfg!(unix) {
            named.extend([PathBuf::from("/tmp"), PathBuf::from("/var/tmp")]);
        }
        if cfg!(windows) {
            named.extend(["TEMP", "TMP"].into_iter().filter_map(std::env::var_os).map(PathBuf::from));
        }
        named.into_iter().map(Directory::new).collect()
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
