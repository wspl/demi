//! The installation's job directories (`runner.md` § Pipes and output): each
//! shell job keeps its kept output, its media and its recorded edits in
//! `<job root>/job-<random>/`. The job root is `jobs/` in a
//! paired device's installation state, and `/var/lib/demi/jobs` on a Cloud's
//! system image. A job's directory lasts until the backend has what it needs
//! of the job: its `job_release`, or, for what a runner that ended left, the
//! next runner's start. A connection loss keeps it, with the job's exit once
//! the job ended, which the next connection reports again (`runner.md`
//! § Command lifetime).

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use demi_runner_protocol::wire;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::job_media::MEDIA_DIRECTORY;
use crate::kept_output::{KeptOutput, KeptReader};

/// The job directories under one installation's job root.
pub struct JobDirectories {
    root: PathBuf,
    /// Each job's directory, from its start until its release or the
    /// connection's end. A `std` mutex, never held across a wait.
    jobs: Mutex<HashMap<String, Held>>,
}

struct Held {
    path: PathBuf,
    output: KeptReader,
    lengths: Arc<StreamLengths>,
    running: bool,
    /// The `job_medium` of each medium the job keeps, in order.
    media: Arc<Mutex<Vec<wire::Frame>>>,
    /// The job's exit and how it ended, once it ended.
    exit: Option<(wire::Frame, wire::KeptEnd)>,
    /// The background tasks that keep the job running once its script has
    /// ended, as its shell names them.
    outliving: Option<watch::Receiver<Vec<String>>>,
}

/// A job's directory and its kept output. The directory counts as running
/// until `running` drops.
pub struct JobDirectory {
    pub path: PathBuf,
    pub output: KeptOutput,
    /// Each stream's length, which the job sets as it writes.
    pub lengths: Arc<StreamLengths>,
    /// The `job_medium` of each medium the job keeps, which it adds as it
    /// announces them.
    pub media: Arc<Mutex<Vec<wire::Frame>>>,
    pub running: Running,
}

/// Each stream's length of a job's output, which a hello reports.
#[derive(Default)]
pub struct StreamLengths {
    stdout: AtomicU64,
    stderr: AtomicU64,
}

impl StreamLengths {
    pub fn set(&self, lengths: wire::OutputLengths) {
        self.stdout.store(lengths.stdout_bytes, Ordering::Relaxed);
        self.stderr.store(lengths.stderr_bytes, Ordering::Relaxed);
    }

    pub fn get(&self) -> wire::OutputLengths {
        wire::OutputLengths {
            stdout_bytes: self.stdout.load(Ordering::Relaxed),
            stderr_bytes: self.stderr.load(Ordering::Relaxed),
        }
    }
}

/// A running job's hold on its directory, which a release keeps.
pub struct Running {
    directories: Arc<JobDirectories>,
    job: String,
}

impl Running {
    /// The job's shell names the tasks that outlive its script in
    /// `outliving`, which a hello lists while the job runs.
    pub fn outlived_by(&self, outliving: watch::Receiver<Vec<String>>) {
        if let Some(held) = self.directories.lock().get_mut(&self.job) {
            held.outliving = Some(outliving);
        }
    }
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
        let lengths = Arc::new(StreamLengths::default());
        let media = Arc::new(Mutex::new(Vec::new()));
        self.lock().insert(
            job.to_owned(),
            Held {
                path: path.clone(),
                output: output.reader(),
                lengths: lengths.clone(),
                running: true,
                media: media.clone(),
                exit: None,
                outliving: None,
            },
        );
        Ok(JobDirectory {
            running: Running {
                directories: self.clone(),
                job: job.to_owned(),
            },
            path,
            output,
            lengths,
            media,
        })
    }

    /// Keeps `job`'s exit, `frame`, and how it ended with its directory,
    /// until the backend releases it; a job without a directory keeps
    /// nothing.
    pub fn ended(&self, job: &str, frame: wire::Frame, end: wire::KeptEnd) {
        if let Some(held) = self.lock().get_mut(job) {
            held.exit = Some((frame, end));
        }
    }

    /// The jobs whose directories are kept, as a hello lists them: each
    /// one's end once it ended, its streams' lengths, how many media it
    /// keeps and, while it runs, the tasks that outlive its script.
    /// `running` names the jobs that run, also one whose directory is not
    /// made yet.
    pub fn kept(&self, running: &[String]) -> Vec<wire::KeptJob> {
        let jobs = self.lock();
        let mut kept: Vec<wire::KeptJob> = jobs
            .iter()
            .map(|(job, held)| wire::KeptJob {
                job_id: job.clone(),
                ended: held.exit.as_ref().map(|(_, end)| end.clone()),
                output: held.lengths.get(),
                media: media_count(&held.path.join(MEDIA_DIRECTORY)),
                outliving: match (&held.exit, &held.outliving) {
                    (None, Some(outliving)) => crate::tasks::wire_tasks(&outliving.borrow()),
                    _ => Vec::new(),
                },
            })
            .collect();
        for job in running {
            if !jobs.contains_key(job) {
                kept.push(wire::KeptJob {
                    job_id: job.clone(),
                    ended: None,
                    output: wire::OutputLengths {
                        stdout_bytes: 0,
                        stderr_bytes: 0,
                    },
                    media: 0,
                    outliving: Vec::new(),
                });
            }
        }
        kept
    }

    /// What a new connection hears again of the jobs not released yet, so
    /// that it misses nothing a lost connection carried: each job's
    /// `job_medium`s, then its exit once it ended (`runner.md` § Command
    /// lifetime).
    pub fn announcements(&self) -> Vec<wire::Frame> {
        let mut frames = Vec::new();
        for held in self.lock().values() {
            frames.extend(
                held.media
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .iter()
                    .cloned(),
            );
            frames.extend(held.exit.as_ref().map(|(frame, _)| frame.clone()));
        }
        frames
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
    /// run: at the runner's start, what an earlier runner left.
    async fn clear(&self) {
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

/// How many media a job keeps in `directory`: none before its first.
fn media_count(directory: &Path) -> u32 {
    std::fs::read_dir(directory)
        .map(|entries| u32::try_from(entries.flatten().count()).unwrap_or(u32::MAX))
        .unwrap_or(0)
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
