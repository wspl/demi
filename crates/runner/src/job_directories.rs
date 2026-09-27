//! The installation's job directories (`runner.md` § Pipes and output): each
//! shell job keeps its whole output, its recorded edits and its scratch
//! directory in `jobs/<conversation>/job-<random>/`, under the conversation
//! its command context names, in lowercase. A job's directory outlives the
//! job, so a tool result can name a file in it, until the conversation's
//! release removes the conversation's directories (`resource-lifecycle.md`
//! § Conversation release). The runner's `hello` names the conversations it
//! holds directories for.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use demi_command_service::protocol::conversation_name;

/// The job directories under one installation's `jobs/`.
pub struct JobDirectories {
    root: PathBuf,
    /// The directories of the jobs that run. A `std` mutex: making a job's
    /// directory and removing a conversation's directories hold it on a
    /// blocking thread, so a release never removes a directory a job is
    /// making or using.
    running: Mutex<HashSet<PathBuf>>,
}

/// A job's directory, for its logs and change records, and the scratch
/// directory its `TMPDIR` names, which goes when the job ends. The directory
/// counts as running until `running` drops.
pub struct JobDirectory {
    pub path: PathBuf,
    pub scratch: tempfile::TempDir,
    pub running: Running,
}

/// A running job's hold on its directory, which a release keeps.
pub struct Running {
    directories: Arc<JobDirectories>,
    path: PathBuf,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.directories.lock().remove(&self.path);
    }
}

impl JobDirectories {
    /// The directories under `root`, the installation's `jobs/`.
    pub fn new(root: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            root,
            running: Mutex::new(HashSet::new()),
        })
    }

    /// The installation's `jobs/`, where the edit lock lives too.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn lock(&self) -> MutexGuard<'_, HashSet<PathBuf>> {
        // No section panics while it holds the lock, so a poisoned one still
        // holds a whole set.
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes a job's directory, private to its user, under the directory of
    /// `conversation`, which the wire checked is a conversation's name.
    pub async fn create(self: &Arc<Self>, conversation: &str) -> io::Result<JobDirectory> {
        let directories = self.clone();
        let parent = self.root.join(conversation.to_ascii_lowercase());
        tokio::task::spawn_blocking(move || {
            let mut running = directories.lock();
            std::fs::create_dir_all(&parent)?;
            let mut builder = tempfile::Builder::new();
            builder.prefix("job-");
            // For the owner alone; Windows directories take their access
            // from the parent's ACL.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                builder.permissions(std::fs::Permissions::from_mode(0o700));
            }
            let path = builder.tempdir_in(&parent)?.keep();
            let scratch = tempfile::Builder::new().prefix(".work-").tempdir_in(&path)?;
            running.insert(path.clone());
            drop(running);
            Ok(JobDirectory {
                running: Running {
                    directories: directories.clone(),
                    path: path.clone(),
                },
                path,
                scratch,
            })
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Removes the directories of `conversation`'s jobs, except those of the
    /// jobs that run, and the conversation's directory once it is empty. A
    /// directory that cannot be removed is logged with the reason and left
    /// for the next release.
    pub async fn release(self: &Arc<Self>, conversation: &str) {
        let directories = self.clone();
        let parent = self.root.join(conversation.to_ascii_lowercase());
        let released = tokio::task::spawn_blocking(move || {
            let running = directories.lock();
            let entries = match std::fs::read_dir(&parent) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return,
                Err(error) => {
                    tracing::warn!(directory = %parent.display(), "job directories not released: {error}");
                    return;
                }
            };
            for entry in entries {
                let path = match entry {
                    Ok(entry) => entry.path(),
                    Err(error) => {
                        tracing::warn!(directory = %parent.display(), "a job directory not released: {error}");
                        continue;
                    }
                };
                if running.contains(&path) {
                    continue;
                }
                if let Err(error) = std::fs::remove_dir_all(&path) {
                    tracing::warn!(directory = %path.display(), "a job directory not released: {error}");
                }
            }
            // A directory still holding a running job's stays.
            if let Err(error) = std::fs::remove_dir(&parent)
                && error.kind() != io::ErrorKind::DirectoryNotEmpty
            {
                tracing::warn!(directory = %parent.display(), "a conversation's job directory not released: {error}");
            }
        })
        .await;
        if let Err(error) = released {
            tracing::warn!("job directories not released: {error}");
        }
    }

    /// The conversations the installation holds job directories for: each
    /// directory under the root whose name is a conversation's.
    pub async fn conversations(&self) -> io::Result<Vec<String>> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let entries = match std::fs::read_dir(&root) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(error) => return Err(error),
            };
            let mut conversations = Vec::new();
            for entry in entries {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if conversation_name(&name, &()).is_ok() {
                    conversations.push(name);
                }
            }
            conversations.sort();
            Ok(conversations)
        })
        .await
        .map_err(io::Error::other)?
    }
}
