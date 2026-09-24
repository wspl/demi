//! The Host contract (`runner.md` § Host operations): an execution target's
//! filesystem and processes, as the agent, the providers and the backend
//! reach them. A Host is shard-local, so its futures and streams of control
//! are not `Send`; file contents are, so a byte path can leave the shard.

use std::{collections::BTreeMap, fmt, sync::Arc};

use bytes::Bytes;
use demi_core::{StreamKind, Timestamp};
use futures_util::{
    future::LocalBoxFuture,
    stream::{BoxStream, LocalBoxStream},
};

/// The value identity of an execution target: equal keys mean the same Host,
/// so state kept per Host, such as an agent's shell environments, is found
/// again through a Host made anew. Whoever makes a Host composes its key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HostKey(Arc<str>);

impl HostKey {
    pub fn new(key: impl Into<Arc<str>>) -> Self {
        Self(key.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HostKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The account a Host works as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostIdentity {
    pub uid: u32,
    pub gid: u32,
    pub hostname: String,
    /// Where the account's home directory is on the Host.
    pub home_dir: String,
}

/// An execution target. A relative path in any of its operations is
/// relative to [`Host::default_cwd`], where work starts when it names no
/// directory; that directory is no boundary: the Host's account decides which
/// paths an operation reaches.
pub trait Host {
    /// Which Host this is.
    fn key(&self) -> HostKey;

    /// Absolute.
    fn default_cwd(&self) -> &str;

    /// The account the Host works as, as its runner last reported it.
    fn identity(&self) -> HostIdentity;

    fn fs(&self) -> &dyn HostFs;

    fn process(&self) -> &dyn HostProcess;
}

/// Why a Host operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct HostError {
    pub kind: HostErrorKind,
    pub message: String,
}

/// What kind of failure a [`HostError`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostErrorKind {
    /// The operation failed on the Host; `code` names the cause the way the
    /// operating system does, such as `ENOENT`, when there is one.
    Failed { code: Option<String> },
    /// A message the operation needs is over the connection's limit; the
    /// connection itself is fine.
    TooLarge,
    /// The Host is not reachable: its runner is not connected, or went away
    /// while the operation ran.
    Offline,
    /// The Host takes no operations now, for example while its Cloud is
    /// stopping or resetting.
    Unavailable,
    /// The Host answered something other than what was asked.
    Protocol,
    /// The caller gave up on the operation, or a byte stream it depends on
    /// broke.
    Interrupted,
}

impl HostError {
    pub fn new(kind: HostErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// A failure on the Host, with its cause's code when there is one.
    pub fn failed(code: Option<String>, message: impl Into<String>) -> Self {
        Self::new(HostErrorKind::Failed { code }, message)
    }

    pub fn offline(message: impl Into<String>) -> Self {
        Self::new(HostErrorKind::Offline, message)
    }

    pub fn interrupted(message: impl Into<String>) -> Self {
        Self::new(HostErrorKind::Interrupted, message)
    }

    /// The operating system's name of the cause, such as `ENOENT`.
    pub fn code(&self) -> Option<&str> {
        match &self.kind {
            HostErrorKind::Failed { code } => code.as_deref(),
            _ => None,
        }
    }
}

/// A file's bytes as they arrive. The stream is `Send`, so the bytes can be
/// copied wherever its reader runs.
pub type ByteStream = BoxStream<'static, Result<Bytes, HostError>>;

/// Which bytes of a file a read takes: `length` bytes from `offset`, or to
/// the end when `length` is none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ByteRange {
    pub offset: u64,
    pub length: Option<u64>,
}

/// What a write puts in a file: its bytes at once, or a stream of them
/// written as they come.
pub enum FileContents {
    Bytes(Bytes),
    Stream(ByteStream),
}

/// A file's metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub kind: FileKind,
    /// The permission and mode bits.
    pub mode: u32,
    pub size: u64,
    pub modified: Timestamp,
}

/// What a directory entry is. `stat` follows a symbolic link, so only an
/// `lstat` or a listing reports [`FileKind::Symlink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    File,
    Directory,
    Symlink,
    CharacterDevice,
    Fifo,
    Other,
}

/// One entry of a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub kind: FileKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteOptions {
    /// Create the directories the file's path names that do not exist yet.
    pub create_parents: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MkdirOptions {
    /// Create the parents too, and accept a directory that exists.
    pub recursive: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RmOptions {
    /// Remove a directory with everything in it.
    pub recursive: bool,
    /// Accept a path that does not exist.
    pub force: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpOptions {
    /// Copy a directory with everything in it.
    pub recursive: bool,
}

/// A Host's filesystem.
pub trait HostFs {
    /// The whole file.
    fn read_file<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<Bytes, HostError>>;

    /// The bytes of `range`. The file is open once this returns, so a
    /// missing or unreadable file fails here, before any byte; dropping the
    /// stream stops the read.
    fn read_stream<'a>(
        &'a self,
        path: &'a str,
        range: ByteRange,
    ) -> LocalBoxFuture<'a, Result<ByteStream, HostError>>;

    /// Replaces the file whole. A write that fails, a stream that fails, or
    /// dropping the future leaves the file as it was; the stream's own error
    /// is the one returned.
    fn write_file<'a>(
        &'a self,
        path: &'a str,
        contents: FileContents,
        options: WriteOptions,
    ) -> LocalBoxFuture<'a, Result<(), HostError>>;

    fn exists<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<bool, HostError>>;

    fn stat<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<FileStat, HostError>>;

    /// The metadata of a symbolic link itself.
    fn lstat<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<FileStat, HostError>>;

    fn read_dir<'a>(
        &'a self,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<Vec<DirEntry>, HostError>>;

    fn mkdir<'a>(
        &'a self,
        path: &'a str,
        options: MkdirOptions,
    ) -> LocalBoxFuture<'a, Result<(), HostError>>;

    fn rm<'a>(
        &'a self,
        path: &'a str,
        options: RmOptions,
    ) -> LocalBoxFuture<'a, Result<(), HostError>>;

    fn cp<'a>(
        &'a self,
        path: &'a str,
        destination: &'a str,
        options: CpOptions,
    ) -> LocalBoxFuture<'a, Result<(), HostError>>;

    fn mv<'a>(
        &'a self,
        path: &'a str,
        destination: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), HostError>>;

    fn chmod<'a>(&'a self, path: &'a str, mode: u32) -> LocalBoxFuture<'a, Result<(), HostError>>;

    /// Makes `path` a symbolic link to `target`, which is written as given.
    fn symlink<'a>(
        &'a self,
        target: &'a str,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), HostError>>;

    /// Makes `path` a hard link to `existing`.
    fn link<'a>(
        &'a self,
        existing: &'a str,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), HostError>>;

    fn readlink<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<String, HostError>>;

    /// The absolute path with every symbolic link resolved.
    fn realpath<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<String, HostError>>;

    fn utimes<'a>(
        &'a self,
        path: &'a str,
        accessed: Timestamp,
        modified: Timestamp,
    ) -> LocalBoxFuture<'a, Result<(), HostError>>;
}

/// A Host's processes: programs it starts outside a shell job, such as a
/// provider's CLI.
pub trait HostProcess {
    /// Starts a process. A process that cannot start is not an error here:
    /// its [`Process::exit`] says why ([`ProcessEnd::NotStarted`]).
    fn spawn(&self, request: SpawnRequest) -> LocalBoxFuture<'_, Result<Process, HostError>>;
}

/// A process to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnRequest {
    pub command: String,
    pub args: Vec<String>,
    /// The Host's default working directory when none.
    pub cwd: Option<String>,
    pub env: SpawnEnv,
    /// The process is kept between pieces of work and is not itself work: a
    /// Host that counts activity leaves it out, and may stop with it running.
    pub retained: bool,
}

/// The environment a process starts with (`runner.md` § Host operations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnEnv {
    /// The Host's own environment.
    Inherit,
    /// Exactly these variables.
    Exactly(BTreeMap<String, String>),
    /// The Host's own environment with these changes: a value sets a
    /// variable, none removes it.
    Overlay(BTreeMap<String, Option<String>>),
}

/// A process the Host started.
pub struct Process {
    /// Its standard output and standard error, each chunk in the order the
    /// Host received it; the stream ends when the process has ended.
    pub output: LocalBoxStream<'static, ProcessOutput>,
    /// Writes to the process and signals it. Dropping it before the process
    /// ended asks the Host to kill the process, without waiting.
    pub control: Box<dyn ProcessControl>,
    /// How the process ended.
    pub exit: LocalBoxFuture<'static, ProcessEnd>,
}

/// One chunk of a process's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub stream: StreamKind,
    pub bytes: Bytes,
}

/// What a caller does to a running process. Once the process has ended,
/// each is a no-op.
pub trait ProcessControl {
    fn write_stdin(&self, bytes: Bytes) -> LocalBoxFuture<'_, Result<(), HostError>>;

    fn close_stdin(&self) -> LocalBoxFuture<'_, Result<(), HostError>>;

    fn kill(&self, signal: Signal) -> LocalBoxFuture<'_, Result<(), HostError>>;
}

/// How a caller stops a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Signal {
    /// `SIGTERM`: ask the process to end.
    Terminate,
    /// `SIGKILL`: end it.
    Kill,
}

/// How a process ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessEnd {
    /// It exited with this code.
    Exited(i32),
    /// A signal ended it, named the way the Host names it, such as
    /// `SIGTERM`.
    Signalled(String),
    /// It never started.
    NotStarted(SpawnError),
    /// The Host went away before the process's end was known, for this
    /// reason.
    Lost(String),
}

/// Why a process could not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnError {
    pub kind: SpawnErrorKind,
    /// The Host's words about it, when it has some.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpawnErrorKind {
    ExecutableNotFound,
    PermissionDenied,
    CwdUnusable,
    IsDirectory,
    Other,
}

impl fmt::Display for SpawnErrorKind {
    /// The kind as the runner names it, which a shell's message quotes.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ExecutableNotFound => "executable_not_found",
            Self::PermissionDenied => "permission_denied",
            Self::CwdUnusable => "cwd_unusable",
            Self::IsDirectory => "is_directory",
            Self::Other => "other",
        })
    }
}
