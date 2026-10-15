//! Each conversation's scratch directory (`runner.md` § Scratch directory):
//! `<DEMI_HOME>/scratch/<conversation id>/`, made at the conversation's
//! first job on the Host, named by `DEMI_SCRATCH` in each of its jobs, and
//! removed once its conversation no longer exists.

use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
};

/// The variable that names a job's scratch directory.
pub const SCRATCH_ENV: &str = "DEMI_SCRATCH";

/// The directory that holds every conversation's scratch directory.
#[derive(Clone, Debug)]
pub struct Scratch {
    root: PathBuf,
}

impl Scratch {
    /// The scratch directories of the installation in `home`, `DEMI_HOME`.
    pub fn new(home: &Path) -> Self {
        Self {
            root: home.join("scratch"),
        }
    }

    /// The directory that holds every conversation's scratch directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory of `conversation`, a conversation name the wire checked
    /// (`native-runtime.md` § Command context), made when it is not there.
    pub async fn create(&self, conversation: &str) -> io::Result<PathBuf> {
        let directory = self.root.join(conversation);
        tokio::fs::create_dir_all(&directory).await?;
        Ok(directory)
    }

    /// Removes the scratch directory of every conversation not in
    /// `existing`, the directories under the root whose name no existing
    /// conversation has, and says which it removed. Anything else there,
    /// such as a file or a link, is left alone, and a directory that cannot
    /// be removed stays until the next connection tries again.
    pub async fn keep_only(&self, existing: &[String]) -> io::Result<()> {
        let existing: HashSet<&str> = existing.iter().map(String::as_str).collect();
        let mut entries = match tokio::fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            let kept = name.to_str().is_some_and(|name| existing.contains(name));
            if kept || !entry.file_type().await?.is_dir() {
                continue;
            }
            let directory = entry.path();
            match tokio::fs::remove_dir_all(&directory).await {
                Ok(()) => tracing::info!(
                    "removed the scratch directory {} of a conversation that no longer exists",
                    directory.display()
                ),
                Err(error) => tracing::warn!(
                    "the scratch directory {} could not be removed: {error}",
                    directory.display()
                ),
            }
        }
        Ok(())
    }
}
