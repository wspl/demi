//! Each conversation's scratch directory, for the files only the agent needs (a
//! screenshot it looks at, a probe script): `<DEMI_HOME>/scratch/<first 12
//! digits of the conversation's id>/` ([`scratch_name`]), made at the
//! conversation's first job on the Host.
//!
//! It shows in no pill and no Change view, and it is still there an hour later,
//! which `/tmp` does not promise.
//!
//! # Who sees which directory
//!
//! `DEMI_SCRATCH` names the calling agent's own:
//!
//! - the conversation's directory for the root agent;
//! - an `agent-<number>/` folder inside it for each subagent, so two subagents
//!   that both save `shots/before.png` overwrite nothing of each other's, while
//!   the parent still reads what a child saved by the path the child reports.
//!
//! The runner's hello names the scratch root, so what the backend writes into a
//! conversation's scratch goes to that directory whatever a job's environment
//! says.
//!
//! # Lifetime
//!
//! The directory goes with the conversation; archiving keeps it. The hello's
//! answer names the user's conversations that still exist, and the runner
//! removes every scratch directory not among them ([`Scratch::keep_only`]). A
//! Host that was offline when a conversation was deleted therefore cleans up as
//! soon as it connects, with nothing to remember in between.
//!
//! The conversation's recordings (`crates/shared-recording`), in a directory
//! of their own inside it, go sooner: when the conversation is archived or
//! deleted. The release the backend sends then says so
//! ([`Scratch::forget_recordings`]), and the hello's answer names the
//! conversations that are archived, so a Host that was offline then removes
//! them as it connects. Neither depends on a service running: a recording
//! outlives every device and tab it recorded, and the conversation's idleness,
//! its move to another Host and the detaching of a device end none.
//!
//! A backend that cannot read the conversations drops the connection rather
//! than name none, which would remove every directory.

use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
};

use demi_command_protocol::{CommandCaller, scratch_name};

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
    /// name the wire checked (`crates/command-protocol/src/invocation.rs`), made
    /// when it is not there.
    pub async fn create(&self, conversation: &str, caller: &CommandCaller) -> io::Result<PathBuf> {
        let mut directory = self.root.join(scratch_name(conversation));
        if let Some(folder) = caller.scratch_folder() {
            directory.push(folder);
        }
        tokio::fs::create_dir_all(&directory).await?;
        Ok(directory)
    }

    /// Removes the scratch directory of every conversation not in `existing`,
    /// and the recordings of those `archived`.
    ///
    /// It removes the directories under the root whose name no existing
    /// conversation has. Anything else there, such as a file or a link, is left
    /// alone, and a directory that cannot be removed stays until the next
    /// connection tries again.
    pub async fn keep_only(&self, existing: &[String], archived: &[String]) -> io::Result<()> {
        let existing: HashSet<String> = existing.iter().map(|id| scratch_name(id)).collect();
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
                    event = "scratch_directory.removed",
                    directory = %name.to_string_lossy(),
                    reason = "conversation_gone",
                    "the scratch directory of a conversation that no longer exists was removed"
                ),
                Err(error) => tracing::warn!(
                    event = "scratch_directory.remove_failed",
                    directory = %name.to_string_lossy(),
                    %error,
                    "the scratch directory of a conversation that no longer exists could not be removed"
                ),
            }
        }
        for conversation in archived {
            self.forget_recordings(conversation).await;
        }
        Ok(())
    }

    /// The names of the conversations' directories under the root: a
    /// conversation that ever ran a job here has one
    /// ([`demi_command_protocol::scratch_name`]). Empty when there is none.
    pub async fn directories(&self) -> io::Result<HashSet<String>> {
        let mut names = HashSet::new();
        let mut entries = match tokio::fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(names),
            Err(error) => return Err(error),
        };
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                names.insert(name.to_owned());
            }
        }
        Ok(names)
    }

    /// Removes `conversation`'s recordings: it was archived or deleted. One
    /// that cannot be removed stays until the next time the runner connects.
    pub async fn forget_recordings(&self, conversation: &str) {
        let directory = demi_command_protocol::recordings_in(&self.root, conversation);
        match tokio::fs::remove_dir_all(&directory).await {
            Ok(()) => tracing::info!(
                event = "recordings.removed",
                conversation,
                reason = "archived_or_deleted",
                "the recordings of an archived or deleted conversation were removed"
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(event = "recordings.remove_failed", conversation, %error, "the recordings of an archived or deleted conversation could not be removed"),
        }
    }
}
