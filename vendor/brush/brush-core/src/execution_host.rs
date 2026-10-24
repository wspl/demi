//! Embedding hooks for job ownership and cancellable machine IO.

use std::{any::Any, fs::File, io, process::Command, sync::Arc, time::Duration};

use crate::{openfiles::OpenFile, processes::ChildProcess};

/// Completion of a redirected output stream owned by the embedding host.
pub type OutputCompletion = std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send + Sync>>;

/// What runs a script file that the system cannot execute by itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptInterpreter {
    /// A program, started with these leading arguments, the script's path and the script's own.
    Program(std::path::PathBuf, Vec<std::ffi::OsString>),
    /// The host's builtin of this name, a shell, which runs the script: it is called with these
    /// leading arguments, the script's path and the script's own, as the shell program would be.
    Builtin(String, Vec<std::ffi::OsString>),
}

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
    /// The control a file gets when its shell moves to `owner`. By default it is `owner`'s; a
    /// control that marks its files (`is_host_output`) keeps the mark on the files it controls.
    fn rebound(&self, owner: &Arc<dyn FileControl>) -> Arc<dyn FileControl> {
        owner.clone()
    }
    /// Whether the files this control controls are the host's own output, which a task that
    /// holds open keeps its host's reader from seeing the end of.
    fn is_host_output(&self) -> bool {
        false
    }
}

/// What a shell's `umask`, `ulimit` and `nohup` set for the processes it starts. A shell
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
    /// Whether the processes the shell starts ignore `SIGHUP`, as `nohup` leaves them; a system
    /// without signals has no such processes to change.
    pub hangup_ignored: bool,
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

/// Processor time, as `time` reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessorTime {
    /// Time spent in user mode.
    pub user: Duration,
    /// Time the system spent on its behalf.
    pub system: Duration,
}

/// What a host measures of one pipeline `time` runs, from its start until it
/// is dropped.
pub trait Timing: Send + Sync {
    /// The processor time of what the pipeline ran so far.
    fn processor_time(&self) -> ProcessorTime;
    /// A line for `time`'s report that says what the measure left out, when
    /// it left something out.
    fn uncounted(&self) -> Option<&'static str>;
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
    /// Whether `file`, a redirection of another stream, is the file `forwarded`
    /// was made of, which that stream's child already writes, so that one
    /// writer serves both.
    fn shares_output(&self, file: &File, forwarded: &File) -> bool {
        let _ = (file, forwarded);
        false
    }
    /// The path a path of the shell's script stands for where the host reads
    /// it otherwise than the system does, such as a `/tmp` that is the user's
    /// temporary folder; `env` gives the shell's variables. With none, the
    /// path is the system's own.
    fn map_path(
        &self,
        path: &std::path::Path,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Option<std::path::PathBuf> {
        let _ = (path, env);
        None
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
    /// Starts measuring a pipeline `time` runs. With none, `time` reports the
    /// processor time of the shell's process and of the children it reaped.
    fn time(&self) -> Option<Box<dyn Timing>> {
        None
    }
    /// Tells the host that a builtin starts, which runs in the shell's process.
    fn builtin_started(&self) {}
    /// What a builtin that runs uucore's own code, as `printf` formats with it,
    /// opens, reads and writes through, as the host's own utilities do, so that
    /// its writes to a file are recorded as theirs are; `attributes` are its
    /// shell's. With none, it opens, reads and writes its files directly.
    fn utility_control(
        &self,
        _attributes: &ChildAttributes,
    ) -> Option<Arc<dyn uucore::context::Control>> {
        None
    }
    /// What runs the file `script`, with `path` the `PATH` the command starts under: a program
    /// or a builtin of the host when the system cannot execute the file by itself, as Windows
    /// cannot a script, or when the host runs it in place of what the system would start. With
    /// none, the system starts `script` itself.
    fn script_interpreter(
        &self,
        script: &std::path::Path,
        path: Option<&std::ffi::OsStr>,
    ) -> io::Result<Option<ScriptInterpreter>> {
        let _ = (script, path);
        Ok(None)
    }
    /// The name of the builtin that stands in for the program at `path`, an absolute path a
    /// command names, so that the command runs that builtin and not the file. With none, the
    /// file is the program.
    fn builtin_at(&self, path: &std::path::Path) -> Option<String> {
        let _ = path;
        None
    }
    /// Whether the host runs the program a bare `name` stands for itself, as one of its builtins,
    /// so that `command -v`, `type` and the like answer with the path of the system's program of
    /// that name where the system has one, as the system's would, and with the name where it has
    /// none.
    fn stands_in_for(&self, name: &str) -> bool {
        let _ = name;
        false
    }
    /// The number `$$` and `BASHPID` give in the shell this host owns, in place of the host
    /// process's id, when the shell is one of the host's own and not the process.
    fn process_id(&self) -> Option<u32> {
        None
    }
    /// Tells the host of a background task that, once the task's own redirections are applied,
    /// it does (`held`) or does not hold the host's own output open.
    fn background_task_output(&self, held: bool) {
        let _ = held;
    }
}

impl OpenFile {
    /// Moves a controlled file to another owner's IO behavior, as a shell that changes hosts
    /// does; other files have none.
    pub(crate) fn rebind(&mut self, owner: &Arc<dyn FileControl>) {
        if let Self::Controlled { control, .. } = self {
            *control = control.rebound(owner);
        }
    }

    /// Whether the file is the host's own output (`FileControl::is_host_output`).
    pub fn is_host_output(&self) -> bool {
        matches!(self, Self::Controlled { control, .. } if control.is_host_output())
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
