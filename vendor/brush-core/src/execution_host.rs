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
}

/// Ownership hooks shared by a shell and every cloned subshell.
pub trait ExecutionHost: Any + Send + Sync {
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
    /// Spawn and retain ownership of an external child.
    fn spawn(&self, command: Command) -> io::Result<ChildProcess>;
    /// Create a pipe; the embedding owner decides how to meet a lack of descriptors.
    fn pipe(&self) -> io::Result<(std::io::PipeReader, std::io::PipeWriter)> {
        std::io::pipe()
    }
    /// Resolve shell-specific path conventions against an explicit cwd.
    fn resolve_path(&self, path: &std::path::Path, cwd: &std::path::Path) -> std::path::PathBuf;
}

impl OpenFile {
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
