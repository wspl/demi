//! The journal of a move between releases (`upgrades.md` § Interruptions):
//! which move, and the step it reached, written durably before each step.
//! A `demi-server` that finds it finishes the move. The release a move goes
//! to writes and reads its journal, so its format is that release's own.

use std::{io, path::Path};

use demi_shared_artifacts::{Mode, Permissions, Publication};
use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Journal {
    pub from: Version,
    pub to: Version,
    pub data: Data,
    pub step: Step,
}

/// What a move does with the backend's databases while both services are
/// stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Data {
    /// An upgrade whose release migrates nothing.
    Keep,
    /// An upgrade copies the databases its release will migrate.
    Snapshot,
    /// A rollback puts back the databases its upgrade copied.
    Restore,
}

/// The steps of a move, in order; each can be taken again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Stopping,
    Data,
    Switching,
    Starting,
}

impl Journal {
    pub fn read(path: &Path) -> io::Result<Option<Self>> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let journal = serde_json::from_slice(&bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {error}", path.display()),
            )
        })?;
        Ok(Some(journal))
    }

    /// Records the journal at `step`, durably, before the step is taken.
    pub fn record(&mut self, path: &Path, step: Step) -> io::Result<()> {
        self.step = step;
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        std::fs::create_dir_all(path.parent().expect("the journal lies in a directory"))?;
        demi_shared_artifacts::publish_bytes_blocking(
            path,
            &bytes,
            Publication {
                mode: Mode::Replace,
                permissions: Permissions::Private,
                durable: true,
            },
        )
        .map_err(io::Error::other)?;
        crate::layout::sync_directory(path.parent().expect("the journal lies in a directory"))
    }

    pub fn remove(path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)?;
        crate::layout::sync_directory(path.parent().expect("the journal lies in a directory"))
    }
}
