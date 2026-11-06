//! The device contract (`crates/runner-device/src/lib.rs`): an execution target's
//! filesystem and processes, as the agent, the providers and the backend
//! reach them. A device is shard-local, so its futures and streams of control
//! are not `Send`; file contents are, so a byte path can leave the shard.

use std::{collections::BTreeMap, fmt, sync::Arc};

use bytes::Bytes;
use demi_shared_types::{ReadAt, StreamKind, Timestamp};
use futures_util::{
    future::LocalBoxFuture,
    stream::{BoxStream, LocalBoxStream},
};

/// The value identity of an execution target: equal keys mean the same device,
/// so state kept per device, such as an agent's shell environments, is found
/// again through a device made anew. Whoever makes a device composes its key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceKey(Arc<str>);

impl DeviceKey {
    pub fn new(key: impl Into<Arc<str>>) -> Self {
        Self(key.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The account a device works as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub uid: u32,
    pub gid: u32,
    pub hostname: String,
    /// Where the account's home directory is on the device.
    pub home_dir: String,
    /// The absolute directory that holds the conversations' scratch
    /// directories on the device, each named by its conversation's id, as the
    /// runner reported it (`crates/runner-jobs/src/scratch.rs`); empty for a device
    /// that never connected.
    pub scratch_root: String,
    /// How the device writes its paths, from the system its runner reports.
    pub paths: demi_device_paths::DevicePaths,
}

/// An execution target. A relative path in any of its operations is
/// relative to [`Device::default_cwd`], where work starts when it names no
/// directory; that directory is no boundary: the device's account decides which
/// paths an operation reaches.
pub trait Device {
    /// Which device this is.
    fn key(&self) -> DeviceKey;

    /// Absolute.
    fn default_cwd(&self) -> &str;

    /// The account the device works as, as its runner last reported it.
    fn identity(&self) -> DeviceIdentity;

    fn fs(&self) -> &dyn DeviceFs;

    fn process(&self) -> &dyn DeviceProcess;
}

/// Why a device operation failed. A failed read the device answered also
/// carries the device's number taken before it read (`crates/runner-device/src/subscriptions.rs`): a file that is not there is an answer like any other.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct DeviceError {
    pub kind: DeviceErrorKind,
    pub message: String,
    pub read_at: Option<ReadAt>,
}

/// Two failures are the same failure whenever the device read them.
impl PartialEq for DeviceError {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.message == other.message
    }
}

impl Eq for DeviceError {}

/// What kind of failure a [`DeviceError`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceErrorKind {
    /// The operation failed on the device; `code` names the cause the way the
    /// operating system does, such as `ENOENT`, when there is one.
    Failed { code: Option<String> },
    /// A message the operation needs is over the connection's limit; the
    /// connection itself is fine.
    TooLarge,
    /// The device is not reachable: its runner is not connected, or went away
    /// while the operation ran.
    Offline,
    /// The device takes no operations now, for example while its Cloud is
    /// stopping or resetting.
    Unavailable,
    /// The device answered something other than what was asked.
    Protocol,
    /// The caller gave up on the operation, or a byte stream it depends on
    /// broke.
    Interrupted,
}

impl DeviceError {
    pub fn new(kind: DeviceErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            read_at: None,
        }
    }

    /// The failure as the device answered a read it took at `read_at`.
    pub fn with_read_at(mut self, read_at: ReadAt) -> Self {
        self.read_at = Some(read_at);
        self
    }

    /// A failure on the device, with its cause's code when there is one.
    pub fn failed(code: Option<String>, message: impl Into<String>) -> Self {
        Self::new(DeviceErrorKind::Failed { code }, message)
    }

    pub fn offline(message: impl Into<String>) -> Self {
        Self::new(DeviceErrorKind::Offline, message)
    }

    pub fn interrupted(message: impl Into<String>) -> Self {
        Self::new(DeviceErrorKind::Interrupted, message)
    }

    /// The operating system's name of the cause, such as `ENOENT`.
    pub fn code(&self) -> Option<&str> {
        match &self.kind {
            DeviceErrorKind::Failed { code } => code.as_deref(),
            _ => None,
        }
    }
}

/// A file's bytes as they arrive. The stream is `Send`, so the bytes can be
/// copied wherever its reader runs.
pub type ByteStream = BoxStream<'static, Result<Bytes, DeviceError>>;

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

/// One entry of a directory, with its own metadata as `lstat` reports it:
/// a symbolic link's size and modification time are the link's, and it
/// carries `link`, where it points. A listing carries them so that it is
/// one request to the device (`crates/runner-device/src/lib.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub kind: FileKind,
    pub size: u64,
    pub modified: Timestamp,
    /// Present for a [`FileKind::Symlink`] only.
    pub link: Option<SymbolicLink>,
}

/// Where a symbolic link points: its target as written, and what is there
/// once every link on the way is followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolicLink {
    pub target: String,
    pub leads_to: LinkTarget,
}

/// What a symbolic link leads to: `Missing` when nothing is there,
/// `Unreadable` with the system's reason when something may be there but
/// cannot be reached (a directory on the way closed to the user, a loop of
/// links, a failing disk).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    File,
    Directory,
    Other,
    Missing,
    Unreadable { reason: String },
}

/// How a file is written. A write makes the directories above its path
/// that are missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteOptions {
    pub exists: WhenExists,
}

/// What a write does when its path is taken. A directory there is never
/// written over: the write fails with `EISDIR`, unless it takes another
/// name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WhenExists {
    /// Replace the file.
    #[default]
    Replace,
    /// Fail with `EEXIST`.
    Refuse,
    /// Take the first free name of the form `name-2.ext`, `name-3.ext`; a
    /// file of the same bytes under one of those names, the given one
    /// first, is the file, and nothing is written.
    Rename,
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

/// A device's filesystem.
pub trait DeviceFs {
    /// The whole file.
    fn read_file<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<Bytes, DeviceError>>;

    /// The bytes of `range`. The file is open once this returns, so a
    /// missing or unreadable file fails here, before any byte; dropping the
    /// stream stops the read.
    fn read_stream<'a>(
        &'a self,
        path: &'a str,
        range: ByteRange,
    ) -> LocalBoxFuture<'a, Result<ByteStream, DeviceError>>;

    /// Writes the file whole and answers the name it has: its path's, or
    /// the free one the write took. A write that fails, a stream that
    /// fails, or dropping the future leaves the file as it was; the stream's
    /// own error is the one returned.
    fn write_file<'a>(
        &'a self,
        path: &'a str,
        contents: FileContents,
        options: WriteOptions,
    ) -> LocalBoxFuture<'a, Result<String, DeviceError>>;

    fn exists<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<bool, DeviceError>>;

    fn stat<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<FileStat, DeviceError>>;

    /// The metadata of a symbolic link itself.
    fn lstat<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<FileStat, DeviceError>>;

    fn read_dir<'a>(
        &'a self,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<Vec<DirEntry>, DeviceError>>;

    fn mkdir<'a>(
        &'a self,
        path: &'a str,
        options: MkdirOptions,
    ) -> LocalBoxFuture<'a, Result<(), DeviceError>>;

    fn rm<'a>(
        &'a self,
        path: &'a str,
        options: RmOptions,
    ) -> LocalBoxFuture<'a, Result<(), DeviceError>>;

    fn cp<'a>(
        &'a self,
        path: &'a str,
        destination: &'a str,
        options: CpOptions,
    ) -> LocalBoxFuture<'a, Result<(), DeviceError>>;

    fn mv<'a>(
        &'a self,
        path: &'a str,
        destination: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), DeviceError>>;

    fn chmod<'a>(&'a self, path: &'a str, mode: u32) -> LocalBoxFuture<'a, Result<(), DeviceError>>;

    /// Makes `path` a symbolic link to `target`, which is written as given.
    fn symlink<'a>(
        &'a self,
        target: &'a str,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), DeviceError>>;

    /// Makes `path` a hard link to `existing`.
    fn link<'a>(
        &'a self,
        existing: &'a str,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), DeviceError>>;

    fn readlink<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<String, DeviceError>>;

    /// The absolute path with every symbolic link resolved.
    fn realpath<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<String, DeviceError>>;

    /// The directory at `path` as [`realpath`](Self::realpath) names it, in
    /// one exchange; `ENOTDIR` when it is no directory. With `create`, one
    /// that does not exist is made first, with its parents.
    fn directory<'a>(&'a self, path: &'a str, create: bool) -> LocalBoxFuture<'a, Result<String, DeviceError>>;

    fn utimes<'a>(
        &'a self,
        path: &'a str,
        accessed: Timestamp,
        modified: Timestamp,
    ) -> LocalBoxFuture<'a, Result<(), DeviceError>>;
}

/// A device's processes: programs it starts outside a shell job, such as a
/// provider's CLI.
pub trait DeviceProcess {
    /// Starts a process. A process that cannot start is not an error here:
    /// its [`Process::exit`] says why ([`ProcessEnd::NotStarted`]).
    fn spawn(&self, request: SpawnRequest) -> LocalBoxFuture<'_, Result<Process, DeviceError>>;
}

/// A process to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnRequest {
    pub command: String,
    pub args: Vec<String>,
    /// The device's default working directory when none.
    pub cwd: Option<String>,
    pub env: SpawnEnv,
    /// Descriptors the process inherits besides its standard streams, each a
    /// pipe that holds its bytes and then ends, such as a token a program
    /// reads from a descriptor rather than from its environment, which
    /// other processes of the same user can read.
    pub descriptors: Vec<Descriptor>,
    /// The process is kept between pieces of work and is not itself work: a
    /// device that counts activity leaves it out, and may stop with it running.
    pub retained: bool,
}

/// A pipe a process inherits at descriptor `fd`, 3 or higher: it reads
/// `bytes`, and then the pipe's end.
#[derive(Clone, PartialEq, Eq)]
pub struct Descriptor {
    pub fd: u32,
    pub bytes: Bytes,
}

impl fmt::Debug for Descriptor {
    /// The bytes may be a secret, so only their length is shown.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Descriptor")
            .field("fd", &self.fd)
            .field("bytes", &format_args!("{} bytes", self.bytes.len()))
            .finish()
    }
}

/// The environment a process starts with (`crates/runner/src/environment.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnEnv {
    /// The device's own environment.
    Inherit,
    /// Exactly these variables.
    Exactly(BTreeMap<String, String>),
    /// The device's own environment with these changes: a value sets a
    /// variable, none removes it.
    Overlay(BTreeMap<String, Option<String>>),
}

/// A process the device started.
pub struct Process {
    /// Its standard output and standard error, each chunk in the order the
    /// device received it; the stream ends when the process has ended.
    pub output: LocalBoxStream<'static, ProcessOutput>,
    /// Writes to the process and signals it. Dropping it before the process
    /// ended asks the device to kill the process, without waiting.
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
    fn write_stdin(&self, bytes: Bytes) -> LocalBoxFuture<'_, Result<(), DeviceError>>;

    fn close_stdin(&self) -> LocalBoxFuture<'_, Result<(), DeviceError>>;

    fn kill(&self, signal: Signal) -> LocalBoxFuture<'_, Result<(), DeviceError>>;
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
    /// A signal ended it, named the way the device names it, such as
    /// `SIGTERM`.
    Signalled(String),
    /// It never started.
    NotStarted(SpawnError),
    /// The device went away before the process's end was known, for this
    /// reason.
    Lost(String),
}

/// Why a process could not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnError {
    pub kind: SpawnErrorKind,
    /// The device's words about it, when it has some.
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
