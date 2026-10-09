//! The installation's job directories (`runner.md` § Pipes and output): each
//! shell job keeps its kept output, its media and its recorded edits in
//! `<job root>/job-<random>/`. The job root is `jobs/` in a
//! paired device's installation state, and `/var/lib/demi/jobs` on a Cloud's
//! system image. A job's directory lasts until the backend has what it needs
//! of the job: its `job_release`, the connection's end, or, for what a
//! runner that ended left, the next connection's start.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio_util::sync::CancellationToken;

use crate::job_media::MEDIA_DIRECTORY;
use crate::kept_output::{KeptOutput, KeptReader};

/// The job directories under one installation's job root, for one
/// connection.
pub struct JobDirectories {
    root: PathBuf,
    /// Each job's directory, from its start until its release or the
    /// connection's end. A `std` mutex, never held across a wait.
    jobs: Mutex<HashMap<String, Held>>,
}

struct Held {
    path: PathBuf,
    output: KeptReader,
    running: bool,
}

/// A job's directory and its kept output. The directory counts as running
/// until `running` drops.
pub struct JobDirectory {
    pub path: PathBuf,
    pub output: KeptOutput,
    pub running: Running,
}

/// A running job's hold on its directory, which a release keeps.
pub struct Running {
    directories: Arc<JobDirectories>,
    job: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(held) = self.directories.lock().get_mut(&self.job) {
            held.running = false;
        }
    }
}

impl JobDirectories {
    /// The directories under `root`, the installation's job root, which
    /// holds none when this returns: a directory there is what a connection
    /// or a runner that ended without its cleanup left, and nothing reads it
    /// any more.
    pub async fn open(root: PathBuf) -> Arc<Self> {
        let directories = Arc::new(Self {
            root,
            jobs: Mutex::new(HashMap::new()),
        });
        directories.clear().await;
        directories
    }

    /// The installation's job root, where the edit lock lives too.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Held>> {
        // No section panics while it holds the lock, so a poisoned one still
        // holds a whole table.
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes `job`'s directory, private to its user, with its kept output.
    pub async fn create(
        self: &Arc<Self>,
        job: &str,
        cancel: &CancellationToken,
    ) -> io::Result<JobDirectory> {
        let root = self.root.clone();
        let path = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&root)?;
            let mut builder = tempfile::Builder::new();
            builder.prefix("job-");
            // For the owner alone; Windows directories take their access
            // from the parent's ACL.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                builder.permissions(std::fs::Permissions::from_mode(0o700));
            }
            Ok::<_, io::Error>(builder.tempdir_in(&root)?.keep())
        })
        .await
        .map_err(io::Error::other)??;
        let output = KeptOutput::create(path.join("output"), cancel).await?;
        self.lock().insert(
            job.to_owned(),
            Held {
                path: path.clone(),
                output: output.reader(),
                running: true,
            },
        );
        Ok(JobDirectory {
            running: Running {
                directories: self.clone(),
                job: job.to_owned(),
            },
            path,
            output,
        })
    }

    /// The kept output of `job`, while its directory lasts.
    pub fn output(&self, job: &str) -> Option<KeptReader> {
        self.lock().get(job).map(|held| held.output.clone())
    }

    /// Where `job` keeps its medium `number`, while its directory lasts
    /// (`crate::job_media`).
    pub fn medium(&self, job: &str, number: u32) -> Option<PathBuf> {
        self.lock()
            .get(job)
            .map(|held| held.path.join(MEDIA_DIRECTORY).join(number.to_string()))
    }

    /// Removes the directory of `job`, which ended. The directory of a job
    /// that runs stays: the backend releases a job only after its end. A
    /// directory that cannot be removed is logged with the reason.
    pub async fn release(&self, job: &str) {
        let path = {
            let mut jobs = self.lock();
            match jobs.get(job) {
                Some(held) if !held.running => jobs.remove(job).map(|held| held.path),
                Some(_) => {
                    tracing::warn!(job, "a running job's directory was not released");
                    None
                }
                None => None,
            }
        };
        if let Some(path) = path {
            remove(path).await;
        }
    }

    /// Removes every directory under the root but those of the jobs that
    /// run: at the connection's start, what an earlier one left, and at its
    /// end, once its jobs ended, all of them.
    pub async fn clear(&self) {
        let running: Vec<PathBuf> = {
            let mut jobs = self.lock();
            jobs.retain(|_, held| held.running);
            jobs.values().map(|held| held.path.clone()).collect()
        };
        let root = self.root.clone();
        let cleared = tokio::task::spawn_blocking(move || {
            let entries = match std::fs::read_dir(&root) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return,
                Err(error) => {
                    tracing::warn!(directory = %root.display(), "the job directories could not be listed: {error}");
                    return;
                }
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && !running.contains(&path)
                    && let Err(error) = std::fs::remove_dir_all(&path)
                {
                    tracing::warn!(directory = %path.display(), "a job directory was not removed: {error}");
                }
            }
        })
        .await;
        if let Err(error) = cleared {
            tracing::warn!("the job directories were not cleared: {error}");
        }
    }
}

async fn remove(path: PathBuf) {
    let removed = tokio::task::spawn_blocking(move || {
        if let Err(error) = std::fs::remove_dir_all(&path)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(directory = %path.display(), "a job directory was not removed: {error}");
        }
    })
    .await;
    if let Err(error) = removed {
        tracing::warn!("a job directory was not removed: {error}");
    }
}
