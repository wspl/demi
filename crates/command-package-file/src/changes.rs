//! Writing the files an edit or a patch changes, together: each is written
//! in turn, and a write that fails restores the files written before it
//! (`commands.md` § File commands).

use std::{fs, path::PathBuf};

use demi_command_sdk::edits::Recording;
use tokio_util::sync::CancellationToken;

use crate::files::{FileError, atomic_write, check_cancelled};

/// One file's change: its bytes before (none for a file it creates) and
/// after (none for a file it deletes), and the permissions a restore gives
/// back to a file it replaces.
pub(crate) struct Change {
    pub(crate) path: PathBuf,
    pub(crate) before: Option<Vec<u8>>,
    pub(crate) after: Option<Vec<u8>>,
    pub(crate) permissions: Option<fs::Permissions>,
}

/// Records and writes `changes` in order. When one fails, the ones written
/// before it are restored, latest first, and their recorded edits undone.
pub(crate) fn commit(
    changes: &[Change],
    cancellation: &CancellationToken,
    mut recording: Option<&mut Recording>,
) -> Result<(), FileError> {
    if let Some(recording) = &mut recording {
        for change in changes {
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
