//! Each conversation's scratch directory, for the files only the agent needs (a
//! screenshot it looks at, a probe script): `<DEMI_HOME>/scratch/<conversation
//! id>/`, made at the conversation's first job on the Host. It shows in no pill and
//! no Change view, and it is still there an hour later, which `/tmp` does not
//! promise. `DEMI_SCRATCH` names the calling agent's own: the conversation's
//! directory for the root agent, an `agent-<number>/` folder inside it for each
//! subagent, so two subagents that both save `shots/before.png` overwrite nothing
//! of each other's, while the parent still reads what a child saved by the path the
//! child reports. The runner's hello names the scratch root, so what the backend
//! writes into a conversation's scratch goes to that directory whatever a job's
//! environment says.
//!
//! It goes with the conversation; archiving keeps it. The hello's answer names the
//! user's conversations that still exist, and the runner removes every scratch
//! directory not among them ([`Scratch::keep_only`]), so a Host that was offline
//! when a conversation was deleted cleans up as soon as it connects, with nothing
//! to remember in between. A backend that cannot read the conversations drops the
//! connection rather than name none, which would remove every directory.

use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
};

use demi_command_protocol::CommandCaller;

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

    /// The scratch directory of `caller` in `conversation`, a conversation
    /// name the wire checked (`native-runtime.md` § Command context), made
    /// when it is not there.
    pub async fn create(&self, conversation: &str, caller: &CommandCaller) -> io::Result<PathBuf> {
        let mut directory = self.root.join(conversation);
        if let Some(folder) = caller.scratch_folder() {
            directory.push(folder);
        }
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
