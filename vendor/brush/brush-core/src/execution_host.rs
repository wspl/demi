//! Embedding hooks for job ownership and cancellable machine IO.

use std::{any::Any, fs::File, io, process::Command, sync::Arc};

use crate::{openfiles::OpenFile, processes::ChildProcess};

/// Completion of a redirected output stream owned by the embedding host.
pub type OutputCompletion = std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send + Sync>>;

/// IO behavior supplied by the embedding execution owner.
pub trait FileControl: Send + Sync {
    /// Read without outliving the owning execution.
    fn read(&self, file: &File, buffer: &mut [u8]) -> io::Result<usize>;
    /// Write without outliving the owning execution.
    fn write(&self, file: &File, buffer: &[u8]) -> io::Result<usize>;
    /// Duplicate a descriptor; the embedding owner decides how to meet a lack of descriptors.
    fn duplicate(&self, file: &File) -> io::Result<File> {
        file.try_clone()
    }
}

/// What a shell's `umask` and `ulimit` set for the processes it starts. A shell
/// embedded in a host process keeps them for itself, since changing that
/// process would change them for everything else it runs; the host applies them
/// to each process the shell starts, before the process runs its program.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChildAttributes {
    /// The file mode creation mask, once the shell set one; otherwise a process
    /// the shell starts inherits the host's.
    pub umask: Option<u32>,
    /// The resource limits the shell set, each resource once, as its soft and
    /// hard limit; a resource not listed keeps what the host gives it.
    #[cfg(unix)]
    pub limits: Vec<(rlimit::Resource, u64, u64)>,
}

/// A background task, a list after `&` or a coprocess, as its host runs it.
pub struct BackgroundTask {
    /// The id the task goes by: `$!` names it, and `jobs -p` lists it.
    pub id: i32,
    /// The host that owns the task's work, from then on the task's shell's.
    pub host: Arc<dyn ExecutionHost>,
    /// Held while the task's list runs.
    pub guard: Box<dyn Send + Sync>,
}

/// Ownership hooks shared by a shell and every cloned subshell.
pub trait ExecutionHost: Any + Send + Sync {
    /// Starts a background task's ownership. With none, the task runs under this host and goes
    /// by no id.
    fn background_task(&self) -> io::Result<Option<BackgroundTask>> {
        Ok(None)
    }
    /// Open a path through the embedding owner's file-operation boundary.
    fn open_file(&self, path: &std::path::Path, options: &std::fs::OpenOptions, writing: bool) -> io::Result<File> {
        let _ = writing;
        options.open(path)
    }
    /// Keep redirected external output on the owner's controlled write path.
    fn external_output(&self, file: File) -> io::Result<(File, Option<OutputCompletion>)> {
        Ok((file, None))
    }
    /// Reject further execution after cancellation.
    fn check(&self) -> io::Result<()>;
    /// Track work until its guard is dropped, including detached interpreter tasks.
    fn task_guard(&self) -> Box<dyn Send + Sync>;
    /// Scope a shell descriptor's reads and writes.
    fn file_control(&self) -> Arc<dyn FileControl>;
    /// Spawn and retain ownership of an external child, giving it the attributes
    /// its shell set before it runs.
    fn spawn(&self, command: Command, attributes: &ChildAttributes) -> io::Result<ChildProcess>;
    /// Create a pipe; the embedding owner decides how to meet a lack of descriptors.
    fn pipe(&self) -> io::Result<(std::io::PipeReader, std::io::PipeWriter)> {
        std::io::pipe()
    }
    /// Create an unnamed temporary file, such as a here-document's; the embedding owner decides
    /// how to meet a lack of descriptors.
    fn temporary_file(&self) -> io::Result<File> {
        tempfile::tempfile()
    }
    /// Resolve shell-specific path conventions against an explicit cwd.
    fn resolve_path(&self, path: &std::path::Path, cwd: &std::path::Path) -> std::path::PathBuf;
}

impl OpenFile {
    /// Moves a controlled file to another owner's IO behavior, as a shell that changes hosts
    /// does; other files have none.
    pub(crate) fn rebind(&mut self, owner: &Arc<dyn FileControl>) {
        if let Self::Controlled { control, .. } = self {
            *control = owner.clone();
        }
    }

    /// Attach the embedding owner's IO behavior to a native file or pipe.
    pub fn controlled(self, control: Arc<dyn FileControl>) -> io::Result<Self> {
        let file = match self {
            Self::Controlled { .. } | Self::Stream(_) => return Ok(self),
            Self::File(file) => file,
            #[cfg(unix)]
            other => {
                let descriptor = other.try_clone_to_owned().map_err(io::Error::other)?;
                descriptor.into()
            }
            #[cfg(windows)]
            other => other.into_file()?,
            #[cfg(not(any(unix, windows)))]
            other => return Ok(other),
        };
        Ok(Self::Controlled { file, control })
    }
}
