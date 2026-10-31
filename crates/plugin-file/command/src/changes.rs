//! Writing the files an edit or a patch changes, together: each is written
//! in turn, and a write that fails restores the files written before it
//! (`crates/plugin-file/command/src/files.rs`).

use std::{fs, path::PathBuf};

use demi_command_sdk::edits::Recording;
use tokio_util::sync::CancellationToken;

use crate::files::{FileError, atomic_write, check_cancelled};

/// One file's change: its bytes before (none for a file it creates) and
/// after (none for a file it deletes), the permissions a restore gives
/// back to a file it replaces, and the path a file it creates was renamed
/// from, whose deletion is another change of the same commit.
pub(crate) struct Change {
    pub(crate) path: PathBuf,
    pub(crate) before: Option<Vec<u8>>,
    pub(crate) after: Option<Vec<u8>>,
    pub(crate) permissions: Option<fs::Permissions>,
    pub(crate) renamed_from: Option<PathBuf>,
}

/// Records and writes `changes` in order. When one fails, the ones written
/// before it are restored, latest first, and their recorded edits undone.
/// Once all are written, each deletion and rename is listed, as `rm` and
/// `mv` list theirs, so the request's files follow them.
pub(crate) fn commit(
    changes: &[Change],
    cancellation: &CancellationToken,
    mut recording: Option<&mut Recording>,
) -> Result<(), FileError> {
    // A rename's new file starts from its old one's tracked text
    // (`Recording::moved`).
    if let Some(recording) = &mut recording {
        for change in changes.iter().filter(|change| change.renamed_from.is_none()) {
            recording.track(&change.path);
        }
    }
    for (index, change) in changes.iter().enumerate() {
        let result = check_cancelled(cancellation).and_then(|()| write(change));
        if let Err(error) = result {
            let mut rollbacks = Vec::new();
            for change in changes[..index].iter().rev() {
                match restore(change) {
                    Err(rollback) => rollbacks.push(rollback),
                    Ok(()) => {
                        if let Some(recording) = &mut recording {
                            recording.restored(&change.path);
                        }
                    }
                }
            }
            if rollbacks.is_empty() {
                return Err(error);
            }
            return Err(FileError::Rollback {
                error: Box::new(error),
                rollbacks,
            });
        }
    }
    if let Some(recording) = recording {
        for change in changes {
            match (&change.after, &change.renamed_from) {
                (Some(_), Some(from)) => recording.moved(from, &change.path),
                (None, _) if !changes.iter().any(|other| other.renamed_from.as_ref() == Some(&change.path)) => {
                    recording.removed(&change.path);
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn write(change: &Change) -> Result<(), FileError> {
    match &change.after {
        Some(bytes) => atomic_write(&change.path, bytes, change.before.is_none()),
        None => Ok(fs::remove_file(&change.path)?),
    }
}

fn restore(change: &Change) -> Result<(), FileError> {
    match &change.before {
        Some(bytes) => {
            atomic_write(&change.path, bytes, change.after.is_none())?;
            if let Some(permissions) = &change.permissions {
                fs::set_permissions(&change.path, permissions.clone())?;
            }
            Ok(())
        }
        None => Ok(fs::remove_file(&change.path)?),
    }
}
