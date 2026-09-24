//! Every message of the runner wire, in both directions, with the values
//! they carry (`runner.md`). Fields are declared in wire order: MessagePack
//! maps are ordered, and a message is written in this order.

use std::collections::BTreeMap;

use demi_command_service::protocol::{
    ArtifactLocation, CommandContext, EditCopies, EditKind, PackageDescriptor, digest, without_nul,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use super::{FsOk, GitOk, LOG_READ_LINES, SERVICE_STDERR_CHARS, Timestamp, WireBytes};
use crate::values::DeviceToken;

/// A message from the backend to the runner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[garde(allow_unvalidated)]
pub enum Inbound {
    ConversationRelease {
        id: String,
        #[garde(length(min = 1))]
        conversation_id: String,
    },
    HelloOk {
        device_id: String,
    },
    ClaimPending {
        claim_token: String,
    },
    Claimed {
        #[garde(skip)]
        device_token: DeviceToken,
    },
    HelloError {
        code: HelloErrorCode,
        reason: String,
    },
    Ping {},
    /// Flush writable filesystems before the guest is stopped; `sync_done`
    /// answers.
    Sync {
        id: String,
    },
    /// The named backing image is now `bytes` large, or `error` says why it
    /// did not grow.
    VolumeGrown {
        id: String,
        volume: VolumeName,
        #[garde(range(min = 1))]
        bytes: u64,
        #[serde(deserialize_with = "Option::deserialize")]
        error: Option<String>,
    },
    Spawn {
        spawn_id: String,
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        args: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        env: Option<BTreeMap<String, Option<String>>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        inherit_env: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        kill_process_group: Option<bool>,
    },
    SpawnStdin {
        spawn_id: String,
        bytes: WireBytes,
    },
    SpawnStdinEnd {
        spawn_id: String,
    },
    SpawnKill {
        spawn_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        signal: Option<Signal>,
    },
    /// One job: `bash -c script` in `cwd` with exactly `env`. Its declared
    /// commands receive `context`; `stdin` and `stdout` attach the job's fd 0
    /// and fd 1 to pipes whose other ends are elsewhere.
    JobStart {
        job_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[garde(inner(custom(digest)))]
        manifest_hash: Option<String>,
        #[garde(dive)]
        context: CommandContext,
        script: String,
        cwd: String,
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        stdin: Option<PipeRef>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        stdout: Option<PipeRef>,
    },
    JobStdin {
        job_id: String,
        bytes: WireBytes,
    },
    JobStdinEnd {
        job_id: String,
    },
    JobKill {
        job_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        signal: Option<Signal>,
    },
    RpcStdinPull {
        call_id: String,
    },
    /// The call's pipe ends, sent before anything else for the call.
    RpcPipes {
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        stdin: Option<PipeRef>,
        stdout: PipeRef,
    },
    /// The call's standard error view; standard output is the pipe.
    RpcOutput {
        call_id: String,
        bytes: WireBytes,
    },
    /// Follows the stdout pipe's drain, so the process has written everything
    /// before it exits with the code.
    RpcExit {
        call_id: String,
        exit_code: u8,
    },
    /// Open a TCP stream on the device's network between the socket and the
    /// two pipes.
    NetOpen {
        stream_id: String,
        host: String,
        #[garde(range(min = 1))]
        port: u16,
        input: PipeRef,
        output: PipeRef,
    },
    /// Open a user stream: invoke `operation` of `package` in its resident
    /// service.
    ServiceOpen {
        stream_id: String,
        #[garde(dive)]
        context: CommandContext,
        #[garde(dive)]
        package: PackageDescriptor,
        #[garde(length(min = 1))]
        operation: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        args: Option<serde_json::Map<String, serde_json::Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        json: Option<bool>,
        #[garde(length(min = 1))]
        cwd: String,
        input: PipeRef,
        output: PipeRef,
    },
    /// Read the Host's log: up to `limit` lines after `since`, oldest first,
    /// of one `source` when named.
    LogRead {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        since: Option<u64>,
        #[garde(range(min = 1, max = LOG_READ_LINES as u64))]
        limit: u64,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[garde(inner(length(min = 1)))]
        source: Option<String>,
    },
    /// The command manifest for the runner's cache, carried opaque; the
    /// runner verifies it as a [`crate::manifest::Manifest`].
    Manifest {
        manifest: serde_json::Value,
    },
    ArtifactLocation {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[garde(dive)]
        location: Option<ArtifactLocation>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        error: Option<String>,
    },
    /// Stream `length` bytes from `offset`, or to the end, into `output`.
    #[serde(rename = "fs_readFile")]
    FsReadFile {
        id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        offset: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        length: Option<u64>,
        output: PipeRef,
    },
    /// Fill the file from `input`.
    #[serde(rename = "fs_writeFile")]
    FsWriteFile {
        id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        create_parents: Option<bool>,
        input: PipeRef,
    },
    FsExists {
        id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
    },
    FsStat {
        id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
    },
    FsLstat {
        id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
    },
    FsReaddir {
        id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        with_file_types: Option<bool>,
    },
    FsMkdir {
        id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        recursive: Option<bool>,
    },
    FsRm {
        id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        recursive: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        force: Option<bool>,
    },
    FsCp {
        id: String,
        path: String,
        destination: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        recursive: Option<bool>,
    },
    FsMv {
        id: String,
        path: String,
        destination: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
    },
    FsChmod {
        id: String,
        path: String,
        mode: u32,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
    },
    FsSymlink {
        id: String,
        target: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
    },
    FsLink {
        id: String,
        existing_path: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
    },
    FsReadlink {
        id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
    },
    FsRealpath {
        id: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
    },
    FsUtimes {
        id: String,
        path: String,
        atime: Timestamp,
        mtime: Timestamp,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
    },
    /// The working tree's changes under `root`.
    GitChanges {
        id: String,
        root: String,
    },
    /// The file as the last commit has it, streamed whole into `output` after
    /// the reply.
    GitShow {
        id: String,
        root: String,
        path: String,
        output: PipeRef,
    },
}

impl Inbound {
    /// The id of an fs call, which its reply carries.
    pub fn fs_request_id(&self) -> Option<&str> {
        match self {
            Self::FsReadFile { id, .. }
            | Self::FsWriteFile { id, .. }
            | Self::FsExists { id, .. }
            | Self::FsStat { id, .. }
            | Self::FsLstat { id, .. }
            | Self::FsReaddir { id, .. }
            | Self::FsMkdir { id, .. }
            | Self::FsRm { id, .. }
            | Self::FsCp { id, .. }
            | Self::FsMv { id, .. }
            | Self::FsChmod { id, .. }
            | Self::FsSymlink { id, .. }
            | Self::FsLink { id, .. }
            | Self::FsReadlink { id, .. }
            | Self::FsRealpath { id, .. }
            | Self::FsUtimes { id, .. } => Some(id),
            _ => None,
        }
    }

    /// The id of a working-tree call, which its reply carries.
    pub fn git_request_id(&self) -> Option<&str> {
        match self {
            Self::GitChanges { id, .. } | Self::GitShow { id, .. } => Some(id),
            _ => None,
        }
    }
}

/// A message from the runner to the backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[garde(allow_unvalidated)]
pub enum Outbound {
    ConversationReleased {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        error: Option<String>,
    },
    /// Where to fetch an artifact, for the live work it serves.
    ArtifactResolve {
        id: String,
        #[garde(dive)]
        owner: ArtifactOwner,
        #[garde(custom(digest))]
        sha256: String,
        #[garde(custom(demi_command_service::protocol::target))]
        target: String,
    },
    Hello {
        protocol: u32,
        /// Absent on an unclaimed first start.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[garde(skip)]
        device_token: Option<DeviceToken>,
        #[garde(dive)]
        runner: RunnerInfo,
    },
    /// Liveness, with the count of running jobs the idle rule reads.
    Pong {
        jobs: u64,
    },
    SyncDone {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        error: Option<String>,
    },
    /// A writable volume is nearly full: the runner asks for this total size.
    VolumeGrow {
        id: String,
        volume: VolumeName,
        #[garde(range(min = 1))]
        bytes: u64,
    },
    FsOk(FsOk),
    /// A failed fs call; `code` is the errno-style code when there is one.
    FsError {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        code: Option<String>,
        message: String,
    },
    GitOk(#[garde(dive)] GitOk),
    /// A failed working-tree call.
    GitError {
        id: String,
        code: String,
        message: String,
    },
    SpawnOutput {
        spawn_id: String,
        stream: OutputStream,
        bytes: WireBytes,
    },
    SpawnExit {
        spawn_id: String,
        #[serde(deserialize_with = "Option::deserialize")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        signal: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        spawn_error: Option<SpawnError>,
    },
    /// Live output while the job runs, up to the view budget per stream.
    JobOutput {
        job_id: String,
        stream: OutputStream,
        bytes: WireBytes,
    },
    /// A registered leaf's guidance while its invocation is active; none
    /// clears that invocation's.
    JobRunningHint {
        job_id: String,
        invocation_id: String,
        #[serde(deserialize_with = "Option::deserialize")]
        hint: Option<String>,
    },
    JobExit {
        job_id: String,
        #[serde(deserialize_with = "Option::deserialize")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        signal: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        spawn_error: Option<SpawnError>,
        /// The directory the script ended in; absent when bash never ran it.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        output: Option<RetainedOutput>,
        #[garde(length(max = demi_command_service::protocol::EDIT_JOB_FILES), dive)]
        files: Vec<JobFileChange>,
        files_truncated: bool,
    },
    /// An `rpc` command invoked on the target. It names only its job; `stdin`
    /// says whether the process has a pipe on fd 0.
    RpcCall {
        job_id: String,
        call_id: String,
        root: String,
        path: Vec<String>,
        argv: Vec<String>,
        args: serde_json::Map<String, serde_json::Value>,
        json: bool,
        cwd: String,
        env: BTreeMap<String, String>,
        stdin: bool,
    },
    RpcStdin {
        call_id: String,
        bytes: WireBytes,
    },
    RpcStdinEnd {
        call_id: String,
    },
    RpcCancel {
        call_id: String,
    },
    /// This runner's end of a pipe closed: its HTTP exchange completed, or why
    /// it did not.
    PipeDone {
        pipe_id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        error: Option<String>,
    },
    NetOpened {
        stream_id: String,
    },
    NetError {
        stream_id: String,
        code: NetErrorCode,
        message: String,
    },
    ServiceOpened {
        stream_id: String,
    },
    ServiceError {
        stream_id: String,
        code: ServiceErrorCode,
        message: String,
    },
    /// A user stream's invocation completed: its exit code and the bounded
    /// tail of its standard error.
    ServiceDone {
        stream_id: String,
        exit_code: u8,
        #[garde(length(utf16, max = SERVICE_STDERR_CHARS))]
        stderr: String,
    },
    LogLines {
        id: String,
        #[garde(length(max = LOG_READ_LINES), dive)]
        lines: Vec<LogLine>,
        next: u64,
    },
    LogError {
        id: String,
        message: String,
    },
}

/// Why a hello was refused. `already_connected` is the one outcome a runner
/// retries: the token's live connection may be a half-open socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelloErrorCode {
    UnsupportedProtocol,
    UnknownDevice,
    AlreadyConnected,
    Revoked,
    Internal,
}

/// A signal a kill request names (`runner.md` § Host operations): one
/// closed set that raw processes and jobs share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Signal {
    #[serde(rename = "SIGTERM")]
    Terminate,
    #[serde(rename = "SIGKILL")]
    Kill,
    #[serde(rename = "SIGINT")]
    Interrupt,
    #[serde(rename = "SIGHUP")]
    Hangup,
    #[serde(rename = "SIGQUIT")]
    Quit,
    #[serde(rename = "SIGUSR1")]
    User1,
    #[serde(rename = "SIGUSR2")]
    User2,
    #[serde(rename = "SIGSTOP")]
    Stop,
    #[serde(rename = "SIGCONT")]
    Continue,
}

/// A managed guest's writable volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeName {
    System,
    Home,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// Why a `net_open` stream could not connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetErrorCode {
    Refused,
    Unreachable,
    ResolveFailed,
    Timeout,
}

/// Why a `service_open` stream never opened: the package lacks the operation,
/// its service could not start, or the runner turned the stream away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceErrorCode {
    UnknownOperation,
    ServiceFailed,
    Refused,
}

/// One end of a pipe: the id the runner reports `pipe_done` under, and the
/// origin-relative URL its end `PUT`s to or `GET`s from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipeRef {
    pub id: String,
    pub url: String,
}

/// The live work an artifact serves, which authorizes it: a job and the
/// manifest it runs with, or a user stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(untagged)]
pub enum ArtifactOwner {
    Job(#[garde(dive)] JobArtifactOwner),
    Stream(#[garde(skip)] StreamArtifactOwner),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobArtifactOwner {
    #[garde(skip)]
    pub job_id: String,
    #[garde(custom(digest))]
    pub manifest_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StreamArtifactOwner {
    pub stream_id: String,
}

/// What a runner says about itself in its hello.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct RunnerInfo {
    pub name: String,
    pub platform: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[garde(inner(custom(demi_command_service::protocol::target)))]
    pub native_target: Option<String>,
    /// Read at shell creation, so it arrives before any Host use.
    pub identity: HostIdentity,
    /// A runner booted as a managed host's init: it presents its token or is
    /// refused, never paired.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    pub managed: Option<bool>,
}

/// The account the runner works as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostIdentity {
    pub uid: u32,
    pub gid: u32,
    pub hostname: String,
    pub home_dir: String,
}

/// A file's metadata, as `stat` or `lstat` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileStat {
    pub is_file: bool,
    pub is_directory: bool,
    pub is_symbolic_link: bool,
    pub mode: u32,
    pub size: u64,
    pub mtime: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    pub uid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    pub gid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    pub ino: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    pub dev: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    pub nlink: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    pub is_character_device: Option<bool>,
    #[serde(
        rename = "isFIFO",
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub is_fifo: Option<bool>,
}

/// One entry of a directory listing with file types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DirEntry {
    pub name: String,
    pub is_file: bool,
    pub is_directory: bool,
    pub is_symbolic_link: bool,
}

/// A directory listing: names, or entries with file types when asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Readdir {
    Names(Vec<String>),
    Entries(Vec<DirEntry>),
}

/// A working tree's changes, as `git status` lists them. The browser
/// receives it too, beside the directory it lists (`web-api.md` § File text
/// and working tree changes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct GitChanges {
    pub repository: bool,
    /// Always written, null before the first commit.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "demi_core::Nullable<String>")]
    pub head: Option<String>,
    #[garde(dive)]
    pub files: Vec<GitChange>,
    pub truncated: bool,
    pub watched: bool,
}

/// One path `git status` lists in a working tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct GitChange {
    pub path: String,
    /// git's two status letters: the index against HEAD, then the working
    /// tree against the index; `??` for an untracked file.
    #[garde(pattern(r"^(?:[MTADRC][ MTDAR]| [MTDAR]|\?\?|DD|AU|UD|UA|DU|AA|UU)$"))]
    pub status: String,
    pub kind: ChangeKind,
    /// The path before a rename.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub from: Option<String>,
    pub added: u64,
    pub removed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// Why a process could not start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnError {
    pub kind: SpawnErrorKind,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnErrorKind {
    ExecutableNotFound,
    PermissionDenied,
    CwdUnusable,
    IsDirectory,
    Other,
}

/// Where a job's full output lives on the target, and the last bytes of each
/// stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetainedOutput {
    pub stdout_path: String,
    pub stderr_path: String,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_tail: WireBytes,
    pub stderr_tail: WireBytes,
}

/// A file a job changed: its edit record entry and its line counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct JobFileChange {
    #[garde(length(min = 1), custom(without_nul))]
    pub path: String,
    #[garde(skip)]
    pub kind: EditKind,
    #[garde(length(max = demi_command_service::protocol::EDIT_JOB_SEGMENTS as usize), dive)]
    pub edits: Vec<EditCopies>,
    #[garde(skip)]
    pub added: u64,
    #[garde(skip)]
    pub removed: u64,
}

/// One line of the Host's log: when it was written, which source wrote it,
/// the conversation the work belonged to when there was one, and the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct LogLine {
    pub at: Timestamp,
    #[garde(length(min = 1))]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[garde(inner(length(min = 1)))]
    pub conversation_id: Option<String>,
    pub text: String,
}

// Closed sets display as the wire spells them, for logs.
serde_plain::derive_display_from_serialize!(HelloErrorCode);
serde_plain::derive_display_from_serialize!(Signal);
serde_plain::derive_display_from_serialize!(VolumeName);
serde_plain::derive_display_from_serialize!(OutputStream);
serde_plain::derive_display_from_serialize!(NetErrorCode);
serde_plain::derive_display_from_serialize!(ServiceErrorCode);
serde_plain::derive_display_from_serialize!(ChangeKind);
serde_plain::derive_display_from_serialize!(SpawnErrorKind);

