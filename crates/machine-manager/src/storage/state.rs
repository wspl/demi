//! The format of the manager's state directory (`managed-hosts.md`
//! § Startup and recovery): `<data>/format` names the format its records
//! are in. A release that changes a record changes the format and migrates
//! the records of every earlier format as it starts; a state directory of a
//! format this release does not know, made by a newer one, stops the start.

use std::path::Path;

use demi_machine_manager_protocol::STATE_FORMAT;

use crate::blocking;

use super::durable::sync;

/// A state directory this release cannot read.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(
        "{} holds state of format {found}, and this release reads format {STATE_FORMAT}: return to the release that made it",
        path.display()
    )]
    Unknown { path: std::path::PathBuf, found: String },
}

/// Records this release's format in a state directory that records none,
/// and accepts one that records it.
pub async fn require_format(data: &Path) -> Result<(), FormatError> {
    let data = data.to_owned();
    blocking::run(move |off| {
        let path = data.join("format");
        let format = STATE_FORMAT.to_string();
        match fs_err::read_to_string(&path) {
            Ok(found) if found.trim() == format => Ok(()),
            Ok(found) => Err(FormatError::Unknown {
                path,
                found: found.trim().to_owned(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                super::durable::write_json(off, &path, &STATE_FORMAT)?;
                sync(off, &data)?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new state directory takes this release's format and opens again;
    /// one a newer release made stops the start.
    #[tokio::test]
    async fn a_state_directory_of_another_format_is_refused() {
        let data = tempfile::tempdir().unwrap();
        require_format(data.path()).await.unwrap();
        require_format(data.path()).await.unwrap();
        std::fs::write(data.path().join("format"), "999\n").unwrap();
        let refused = require_format(data.path()).await;
        assert!(matches!(refused, Err(FormatError::Unknown { .. })), "{refused:?}");
    }
}
