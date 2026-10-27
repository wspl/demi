//! Every message of the runner wire, in both directions, with the values
//! they carry (`crates/runner-protocol/src/wire/mod.rs`). Fields are declared in wire order: MessagePack
//! maps are ordered, and a message is written in this order.

use std::collections::BTreeMap;

use demi_command_protocol::{
    ArtifactLocation, CommandContext, EditCopies, EditKind, PackageDescriptor,
    ResidentSequence, Viewable, conversation_name, digest, without_nul,
};
use demi_shared_types::{BlobRef, HostLogLine};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use super::{
    Carriage, FsOk, GitOk, JOB_OUTLIVING_TASKS, JOB_TASK_LINE_BYTES,
    RESIDENT_STDERR_CHARS, Timestamp, WireBytes,
};
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
    /// Ends what the runner's residents hold for the conversation
    /// (`crates/backend-user-shard/src/lifecycle/conversations.rs`).
    ///
    /// `over` says the conversation is archived or deleted, which also ends
    /// what the runner itself keeps for it besides its residents' state: its
    /// recordings.
    ConversationRelease {
        id: String,
        #[garde(custom(conversation_name))]
        conversation_id: String,
        over: bool,
    },
    /// Stops the residents of `packages`, by their ids, which ends what they
    /// hold for every conversation: their plugin was turned off
    /// (`crates/backend-plugins/src/lib.rs`).
    PackageRelease {
        id: String,
        #[garde(inner(length(min = 1)))]
        packages: Vec<String>,
    },
    /// The device the hello's token names, by its id and the name the
    /// user sees, and the user's conversations that still exist, archived
    /// ones included, whose scratch directories the runner keeps, and which of
    /// them are archived, whose recordings it removes
    /// (`crates/runner-jobs/src/scratch.rs`), which of them it is to hold
    /// residents for, and the packages whose residents it is to hold none of
    /// (`crates/runner/src/connection/owner.rs`).
    HelloOk {
        device_id: String,
        device_name: String,
        #[garde(inner(custom(conversation_name)))]
        conversations: Vec<String>,
        #[garde(inner(custom(conversation_name)))]
        archived: Vec<String>,
        /// The conversations the device is to hold residents for: those that
        /// are neither archived, nor detached from it, nor released after
        /// idling. The runner releases every other it holds one for.
        #[garde(inner(custom(conversation_name)))]
        held: Vec<String>,
        /// The packages whose plugin the user has turned off, whose residents
        /// the runner stops. Not a list of those to run, since the runner
        /// also runs packages no plugin binds.
        stopped: Vec<String>,
        /// The number of the session this connection carries, which the
        /// backend drew when it started the session and the runner's next hello
        /// names to resume it (`crates/runner-protocol/src/wire/session.rs`).
        /// A number other than the one the runner holds is a new session.
        session: u64,
        /// How many of the runner's numbered messages the backend received
        /// in the session this connection carries on; none when the session
        /// is a new one.
        received: u64,
    },
    ClaimPending {
        claim_token: String,
    },
    /// The runner was paired: its device token, the name the user sees, and
    /// the number of the session it begins, which the runner's next hello
    /// resumes, as it does for a `hello_ok`'s.
    Claimed {
        #[garde(skip)]
        device_token: DeviceToken,
        device_name: String,
        session: u64,
    },
    HelloError {
        code: HelloErrorCode,
        reason: String,
    },
    /// The user revoked the device, or the backend did at the runner's
    /// `revoke`: the runner removes itself from it, and the backend closes
    /// the connection (`crates/runner/src/removal.rs`).
    /// `projects` names the projects that went with the device, whose files
    /// stay.
    Revoked {
        projects: Vec<String>,
    },
    Ping {},
    /// The runner release the backend moves to next, and its executable
    /// for the runner's target as a 409 would name it: a paired device's
    /// runner downloads it into its installation while it serves, and the
    /// programs of the release's `packages` into its artifact cache, keeping
    /// the ones it runs now, and answers `prefetched` or `prefetch_failed`
    /// (`crates/runner/src/update.rs`).
    Prefetch {
        #[garde(custom(digest))]
        release: String,
        #[garde(custom(digest))]
        sha256: String,
        #[garde(range(min = 1))]
        size: u64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[garde(dive)]
        packages: Vec<PackageDescriptor>,
    },
    /// The command packages of the backend's release, sent once the
    /// connection opens: the runner puts the program of each for its target
    /// in its artifact cache, so no command waits for its download
    /// (`crates/runner/src/connection/owner.rs`).
    Programs {
        #[garde(dive)]
        packages: Vec<PackageDescriptor>,
    },
    /// Flush writable filesystems before the guest is stopped; `sync_done`
    /// answers.
    Sync {
        id: String,
    },
    Spawn {
        spawn_id: String,
        command: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        args: Option<Vec<String>>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        env: Option<BTreeMap<String, Option<String>>>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        inherit_env: Option<bool>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        kill_process_group: Option<bool>,
        /// Pipes the process inherits besides its standard streams, each
        /// holding its bytes and then ending.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        descriptors: Option<Vec<Descriptor>>,
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
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        signal: Option<Signal>,
    },
    /// One job: `bash -c script` in `cwd` with exactly `env`. A `cwd` that does
    /// not exist fails it before its script runs.
    ///
    /// - Its declared commands receive `context` and may show the model the
    ///   media types `viewable` names, none for a job whose media reach no
    ///   model, as one `demi host shell` runs.
    /// - `stdin` and `stdout` attach the job's fd 0 and fd 1 to pipes whose
    ///   other ends are elsewhere.
    JobStart {
        job_id: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[garde(inner(custom(digest)))]
        manifest_hash: Option<String>,
        #[garde(dive)]
        context: CommandContext,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[garde(dive)]
        viewable: Option<Viewable>,
        script: String,
        cwd: String,
        env: BTreeMap<String, String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        stdin: Option<PipeRef>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
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
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        signal: Option<Signal>,
    },
    /// Starts or stops sending the job's output beyond each stream's first
    /// `JOB_VIEW_BYTES` (`crates/runner-jobs/src/tasks.rs`). A job starts
    /// unfollowed.
    JobFollow {
        job_id: String,
        follow: bool,
    },
    /// Streams the job's kept output, as it stands, into `output`; the
    /// runner answers with `job_read` (`crates/runner-jobs/src/kept_output.rs`).
    JobRead {
        id: String,
        job_id: String,
        output: PipeRef,
    },
    /// Streams the media `numbers` the job keeps into `output`, each after
    /// its length as [`FileRead`] describes; the runner answers with
    /// `job_media_read` (`crates/runner-jobs/src/job_media.rs`).
    JobMediaRead {
        id: String,
        job_id: String,
        #[garde(length(min = 1), inner(range(min = 1)))]
        numbers: Vec<u32>,
        output: PipeRef,
    },
    /// The backend has read what it needs of an ended job: its directory
    /// goes.
    JobRelease {
        job_id: String,
    },
    RpcStdinPull {
        call_id: String,
    },
    /// The call's pipe ends, sent before anything else for the call.
    RpcPipes {
        call_id: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
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
    /// Open a user stream: invoke `operation` of `package` in its resident
    /// resident.
    ResidentOpen {
        stream_id: String,
        #[garde(dive)]
        context: CommandContext,
        #[garde(dive)]
        package: PackageDescriptor,
        #[garde(length(min = 1))]
        operation: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        args: Option<serde_json::Map<String, serde_json::Value>>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        json: Option<bool>,
        #[garde(length(min = 1))]
        cwd: String,
        input: PipeRef,
        output: PipeRef,
    },
    /// The backend gave up the user stream `stream_id`, opened or still
    /// opening: the runner ends it.
    ResidentClose {
        stream_id: String,
    },
    /// Send the Host's log lines after `since`, then each new line as it is
    /// written: the highest `seq` the backend stored for this Host, none for
    /// a Host it has no line of.
    LogFollow {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        since: Option<u64>,
    },
    /// The command manifest for the runner's cache, carried opaque; the
    /// runner verifies it as a [`crate::manifest::Manifest`].
    Manifest {
        manifest: serde_json::Value,
    },
    ArtifactLocation {
        id: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[garde(dive)]
        location: Option<ArtifactLocation>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        error: Option<String>,
    },
    /// The numbers a `numbers_reserve` asked for: the first of them, or why
    /// there are none (`crates/command-protocol/src/numbers.rs`).
    NumbersReserved {
        id: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[garde(inner(range(min = 1)))]
        first: Option<u64>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[garde(inner(length(min = 1)))]
        error: Option<String>,
    },
    /// Stream `length` bytes from `offset`, or to the end, into `output`.
    /// A request that names the `version` it holds streams nothing while
    /// the file still has it: the reply says it is unchanged
    /// (`crates/runner-host/src/files.rs`).
    #[serde(rename = "fs_readFile")]
    FsReadFile {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        offset: Option<u64>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        length: Option<u64>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        version: Option<String>,
        output: PipeRef,
    },
    /// The file's size and SHA-256, and none of its bytes, so the backend
    /// can tell whether it holds them already; a file over `limit` bytes
    /// answers its size alone, before a byte is read (`crates/runner-host/src/files.rs`).
    #[serde(rename = "fs_hashFile")]
    FsHashFile {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
        limit: u64,
    },
    /// Fill the file from `input`, making the directories above it that are
    /// missing; `exists` says what to do when the path is taken. The answer
    /// names the file it wrote.
    #[serde(rename = "fs_writeFile")]
    FsWriteFile {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
        exists: WriteExists,
        input: PipeRef,
    },
    /// Fill the file with the bytes the backend offers at `source`.
    ///
    /// The runner downloads them as every file Demi downloads
    /// (`crates/shared-artifacts/src/download.rs`): resumed after a break or a
    /// stall, with no overall deadline, and checked against the offer's size
    /// and SHA-256 before the file is put in place as `fs_writeFile` puts it.
    /// The answer names the file it wrote, or the cause of the failure.
    #[serde(rename = "fs_downloadFile")]
    FsDownloadFile {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
        exists: WriteExists,
        source: FileOffer,
    },
    /// Writes the directory `path` whole (`crates/runner-host/src/lib.rs`).
    ///
    /// - `files` lists its files, each by its path inside it, with `/` between
    ///   the parts, and its mode.
    /// - `input` carries their contents in that order, each after its length as
    ///   [`FileRead`] describes.
    /// - The runner writes them into a temporary directory beside `path`, gives
    ///   every directory in it `directory_mode`, and renames it into place.
    /// - A directory at `path` already is complete, and is kept.
    #[serde(rename = "fs_writeDirectory")]
    FsWriteDirectory {
        id: String,
        path: String,
        #[garde(length(min = 1))]
        files: Vec<DirectoryFile>,
        directory_mode: u32,
        input: PipeRef,
    },
    /// Removes each of `paths` with everything in it, a read-only directory
    /// made writable first; a path that is not there is removed already.
    #[serde(rename = "fs_removeAll")]
    FsRemoveAll {
        id: String,
        #[garde(length(min = 1))]
        paths: Vec<String>,
    },
    /// Reads several files at once (`crates/runner-host/src/lib.rs`): the
    /// answer says of each whether it is read, and `output` carries the
    /// read ones' contents, each after its length, in the request's order.
    /// A file over `limit` bytes is not read.
    #[serde(rename = "fs_readFiles")]
    FsReadFiles {
        id: String,
        #[garde(length(min = 1))]
        paths: Vec<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
        limit: u64,
        output: PipeRef,
    },
    /// Searches the text of the files under `root`
    /// (`crates/runner-host/src/search.rs`). The reply comes once the query is
    /// read and `root` is a directory; then `output` carries what the search
    /// finds, one [`crate::search::SearchLine`] per line, until it ends or the
    /// pipe closes.
    FsSearch {
        id: String,
        #[garde(length(min = 1))]
        root: String,
        #[garde(dive)]
        query: crate::search::SearchQuery,
        output: PipeRef,
    },
    /// Looks at several paths at once (`crates/runner-host/src/lib.rs`):
    /// each answers whether it exists, its kind, a directory's entries, or a
    /// file's size with its first bytes, up to the path's `limit`, which go
    /// into `output` one file after another in the request's order. Without
    /// `output`, no file's bytes are read.
    FsLook {
        id: String,
        #[garde(length(min = 1))]
        paths: Vec<LookPath>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        output: Option<PipeRef>,
    },
    FsExists {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    FsStat {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    FsLstat {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    /// Lists a directory, each entry with its file type.
    FsReaddir {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    FsMkdir {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        recursive: Option<bool>,
    },
    FsRm {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        recursive: Option<bool>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        force: Option<bool>,
    },
    FsCp {
        id: String,
        path: String,
        destination: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        recursive: Option<bool>,
    },
    FsMv {
        id: String,
        path: String,
        destination: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    FsChmod {
        id: String,
        path: String,
        mode: u32,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    FsSymlink {
        id: String,
        target: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    FsLink {
        id: String,
        existing_path: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    FsReadlink {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    FsRealpath {
        id: String,
        path: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    /// The directory at `path` as its Host names it, in one exchange
    /// (`crates/backend-host-access/src/directories.rs`): its real path, or
    /// `ENOTDIR` when it is no directory. With `create`, a directory that
    /// does not exist is made first, with its parents, and a path that cannot
    /// be one fails with a reason the runner words from the part of it that
    /// exists: that part is a file, or the drive is missing.
    FsDirectory {
        id: String,
        path: String,
        #[serde(default)]
        create: bool,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    FsUtimes {
        id: String,
        path: String,
        atime: Timestamp,
        mtime: Timestamp,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        cwd: Option<String>,
    },
    /// The working tree's changes under `root`.
    GitChanges {
        id: String,
        root: String,
    },
    /// The file as the last commit has it, or as `commit` has it, streamed
    /// whole into `output` after the reply.
    GitShow {
        id: String,
        root: String,
        path: String,
        output: PipeRef,
        /// An id or an abbreviation of at least 7 hex digits; HEAD without one.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        commit: Option<String>,
    },
    /// The local branches of `root`'s repository and the one checked out.
    GitBranches {
        id: String,
        root: String,
    },
    /// One page of a branch's first-parent history, HEAD's without
    /// `branch`, continuing after the commit `after` names.
    GitLog {
        id: String,
        root: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        branch: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        after: Option<String>,
    },
    /// One commit: its entry, its files against its first parent and the
    /// local branches it is on.
    GitCommit {
        id: String,
        root: String,
        commit: String,
    },
    /// Subscribes `id` to what `kind` says of `path` (`crates/runner-host/src/subscriptions.rs`): `fs_live` answers once the watch behind it runs, or
    /// `fs_failed` when it cannot, and `fs_changed` names it when its
    /// subject changed. Its lease runs 10 minutes from now; a subscription
    /// under an id that runs already is replaced.
    FsSubscribe {
        #[garde(length(min = 1))]
        id: String,
        kind: crate::files::SubscriptionKind,
        #[garde(length(min = 1), custom(without_nul))]
        path: String,
        /// See [`crate::files::Subject::suffix`].
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        suffix: Option<String>,
    },
    /// Ends subscription `id` at once; one that ended already is ended.
    FsUnsubscribe {
        id: String,
    },
    /// Renews the leases of `ids` for 10 minutes from now; an id that ended
    /// renews nothing.
    FsRenew {
        ids: Vec<String>,
    },
    /// Introduces a page to the runner (`crates/runner-direct/src/lib.rs`): `sdp` is the page's offer for peer `peer`, which replaces
    /// the peer of that id; `direct_answer` or `direct_refused` answers.
    DirectOffer {
        id: String,
        #[garde(length(min = 1))]
        peer: String,
        #[garde(length(min = 1))]
        sdp: String,
        #[garde(dive)]
        introduction: crate::direct::Introduced,
        /// The STUN servers the runner asks, from its local network
        /// sockets, for the address the internet sees it at; none keeps
        /// the peer to one network.
        #[garde(length(max = crate::direct::MAX_STUN_URLS))]
        stun: Vec<crate::direct::StunUrl>,
    },
    /// A candidate the page found after its offer, which the runner checks
    /// too (`crates/runner-direct/src/lib.rs`): the `candidate:` line
    /// as the browser gives it.
    DirectCandidate {
        #[garde(length(min = 1))]
        peer: String,
        #[garde(length(min = 1, max = crate::direct::CANDIDATE_CHARS))]
        candidate: String,
    },
    /// A page's probe of the relay path (`packages/web/src/direct/measure.ts`): the runner answers `direct_pong` with the same `id` at once,
    /// whether or not it holds the peer.
    DirectPing {
        #[garde(length(min = 1))]
        peer: String,
        #[garde(skip)]
        id: u32,
    },
    /// The page's signaling lane closed, or the other runner's peer
    /// ended: the runner closes the peer.
    DirectClose {
        #[garde(length(min = 1))]
        peer: String,
    },
    /// The other runner's answer to this runner's `direct_dial`, with its
    /// candidates; its later ones come as `direct_candidate`.
    DirectDialed {
        id: String,
        #[garde(length(min = 1))]
        sdp: String,
    },
    /// The backend did not introduce this runner to the other.
    DirectDialRefused {
        id: String,
        code: crate::direct::DialRefusal,
        message: String,
    },
    /// Every forward this runner is an end of (`crates/backend-user-shard/src/shard/port_forwards.rs`), as
    /// the connection begins and whenever one of them, or the presence of
    /// a device at its other end, changes.
    Forwards {
        #[garde(dive)]
        forwards: Vec<crate::forward::Forward>,
        /// The STUN servers the runner asks, as a page's peer does, when it
        /// offers a peer to another runner.
        #[garde(length(max = crate::direct::MAX_STUN_URLS))]
        stun: Vec<crate::direct::StunUrl>,
    },
    /// The directories of the projects on this runner's Host, as the
    /// backend stores them, as the connection begins and whenever one is
    /// added, removed or named otherwise. The runner reports each one's
    /// linked worktrees (`worktrees`) and keeps them current
    /// (`crates/runner-host/src/worktrees.rs`).
    Projects {
        #[garde(inner(length(min = 1)))]
        directories: Vec<String>,
    },
    /// The user asks the Listen On runner to try forward `forward`'s port
    /// again.
    ForwardRetry {
        #[garde(length(min = 1))]
        forward: String,
    },
    /// A connection of forward `forward` on the relay: the runner connects
    /// to the forward's address, writes what `input` carries to it, and
    /// what it reads into `output`.
    ForwardConnect {
        #[garde(length(min = 1))]
        forward: String,
        input: PipeRef,
        output: PipeRef,
    },
    /// The answer to `forward_open`: the connection's pipes, from the
    /// Listen On runner's side.
    ForwardOpened {
        id: String,
        input: PipeRef,
        output: PipeRef,
    },
    /// The connection is refused at once: the Connect To Host is offline or
    /// stopped, or the forward no longer runs.
    ForwardRefused {
        id: String,
        message: String,
    },
    /// A request the backend makes for the user to exposed port `port`
    /// (`crates/backend-user-shard/src/shard/exposed_ports.rs`): the runner makes it with `method`, `path` and
    /// `headers`, reading its body from `body`, when it has one, and answers `http_answered`
    /// with the status and headers, the answer's body going into `answer`,
    /// or `http_failed`.
    HttpRequest {
        id: String,
        scheme: crate::exposure::HttpScheme,
        #[garde(range(min = 1))]
        port: u16,
        #[garde(length(min = 1, max = 32))]
        method: String,
        /// The path and query, from `/`.
        #[garde(pattern("^/"))]
        path: String,
        #[garde(length(max = crate::exposure::MAX_HEADERS), dive)]
        headers: Vec<crate::exposure::HttpHeader>,
        /// None for a request without a body.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        body: Option<PipeRef>,
        answer: PipeRef,
    },
}

impl Inbound {
    /// Whether the message is a stream of bytes someone wrote, which the
    /// backend writes behind every control message
    /// (`crates/runner-protocol/src/wire/mod.rs`): a command's or a process's
    /// stdin, and the stderr of a backend command a call relays with the exit
    /// that ends it. Such a stream stays in order with itself, so its end comes
    /// after its bytes.
    pub fn is_input(&self) -> bool {
        matches!(
            self,
            Self::SpawnStdin { .. }
                | Self::SpawnStdinEnd { .. }
                | Self::JobStdin { .. }
                | Self::JobStdinEnd { .. }
                | Self::RpcOutput { .. }
                | Self::RpcExit { .. }
        )
    }

    /// How the message travels the link's session
    /// (`crates/runner-protocol/src/wire/session.rs`): a message of the
    /// connection's own, one that goes on across a drop, such as a command's
    /// stdin, a stop or a call's reply, or one that asks something of the
    /// connection it was written to, such as a Host request, which a new
    /// connection must not run late. A job's start is the connection's too:
    /// the backend sends it again itself when the runner's hello does not list
    /// the job, and a second start would run the command twice.
    pub fn carriage(&self) -> Carriage {
        // Every message is named, so a new one is classified when it is added.
        match self {
            Self::HelloOk { .. }
            | Self::HelloError { .. }
            | Self::ClaimPending { .. }
            | Self::Claimed { .. }
            | Self::Revoked { .. }
            | Self::Ping { .. } => Carriage::Connection,
            Self::JobStdin { .. }
            | Self::JobStdinEnd { .. }
            | Self::JobKill { .. }
            | Self::JobFollow { .. }
            | Self::JobRelease { .. }
            | Self::Manifest { .. }
            | Self::RpcStdinPull { .. }
            | Self::RpcPipes { .. }
            | Self::RpcOutput { .. }
            | Self::RpcExit { .. } => Carriage::Outlasting,
            Self::ConversationRelease { .. }
            | Self::PackageRelease { .. }
            | Self::Prefetch { .. }
            | Self::Programs { .. }
            | Self::Sync { .. }
            | Self::Spawn { .. }
            | Self::SpawnStdin { .. }
            | Self::SpawnStdinEnd { .. }
            | Self::SpawnKill { .. }
            | Self::JobStart { .. }
            | Self::JobRead { .. }
            | Self::JobMediaRead { .. }
            | Self::ResidentOpen { .. }
            | Self::ResidentClose { .. }
            | Self::LogFollow { .. }
            | Self::ArtifactLocation { .. }
            | Self::NumbersReserved { .. }
            | Self::FsReadFile { .. }
            | Self::FsHashFile { .. }
            | Self::FsWriteFile { .. }
            | Self::FsDownloadFile { .. }
            | Self::FsWriteDirectory { .. }
            | Self::FsRemoveAll { .. }
            | Self::FsReadFiles { .. }
            | Self::FsSearch { .. }
            | Self::FsLook { .. }
            | Self::FsExists { .. }
            | Self::FsStat { .. }
            | Self::FsLstat { .. }
            | Self::FsReaddir { .. }
            | Self::FsMkdir { .. }
            | Self::FsRm { .. }
            | Self::FsCp { .. }
            | Self::FsMv { .. }
            | Self::FsChmod { .. }
            | Self::FsSymlink { .. }
            | Self::FsLink { .. }
            | Self::FsReadlink { .. }
            | Self::FsRealpath { .. }
            | Self::FsDirectory { .. }
            | Self::FsUtimes { .. }
            | Self::GitChanges { .. }
            | Self::GitShow { .. }
            | Self::GitBranches { .. }
            | Self::GitLog { .. }
            | Self::GitCommit { .. }
            | Self::FsSubscribe { .. }
            | Self::FsUnsubscribe { .. }
            | Self::FsRenew { .. }
            | Self::DirectOffer { .. }
            | Self::DirectCandidate { .. }
            | Self::DirectPing { .. }
            | Self::DirectClose { .. }
            | Self::DirectDialed { .. }
            | Self::DirectDialRefused { .. }
            | Self::Forwards { .. }
            | Self::Projects { .. }
            | Self::ForwardRetry { .. }
            | Self::ForwardConnect { .. }
            | Self::ForwardOpened { .. }
            | Self::ForwardRefused { .. }
            | Self::HttpRequest { .. } => Carriage::Scoped,
        }
    }

    /// The id of an fs call, which its reply carries.
    pub fn fs_request_id(&self) -> Option<&str> {
        match self {
            Self::FsReadFile { id, .. }
            | Self::FsHashFile { id, .. }
            | Self::FsWriteFile { id, .. }
            | Self::FsDownloadFile { id, .. }
            | Self::FsLook { id, .. }
            | Self::FsReadFiles { id, .. }
            | Self::FsSearch { id, .. }
            | Self::FsWriteDirectory { id, .. }
            | Self::FsRemoveAll { id, .. }
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
            | Self::FsDirectory { id, .. }
            | Self::FsUtimes { id, .. } => Some(id),
            _ => None,
        }
    }

    /// The id of a working-tree call, which its reply carries.
    pub fn git_request_id(&self) -> Option<&str> {
        match self {
            Self::GitChanges { id, .. }
            | Self::GitShow { id, .. }
            | Self::GitBranches { id, .. }
            | Self::GitLog { id, .. }
            | Self::GitCommit { id, .. } => Some(id),
            _ => None,
        }
    }
}

/// What a runner's hello says of the session it carries on: the number of the
/// session the backend's last answer named, and how many of the backend's
/// numbered messages it received in it (`crates/runner-protocol/src/wire/session.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resume {
    pub session: u64,
    pub received: u64,
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
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        error: Option<String>,
    },
    /// The residents of a `package_release` ended.
    PackageReleased {
        id: String,
    },
    /// Reserves `count` numbers of the conversation's `sequence` for a
    /// resident (`crates/command-protocol/src/numbers.rs`).
    NumbersReserve {
        id: String,
        #[garde(custom(conversation_name))]
        conversation_id: String,
        sequence: ResidentSequence,
        #[garde(range(min = 1))]
        count: u32,
    },
    /// A message of the resident of `package` to its plugin about the
    /// conversation (`crates/command-protocol/src/messages.rs`).
    PackageMessage {
        #[garde(length(min = 1))]
        package: String,
        #[garde(custom(conversation_name))]
        conversation_id: String,
        message: serde_json::Value,
    },
    /// The resident of `package` ended without releasing its conversations,
    /// so what it kept for them is lost.
    PackageLost {
        #[garde(length(min = 1))]
        package: String,
    },
    /// Where to fetch an artifact, for the live work it serves.
    ArtifactResolve {
        id: String,
        #[garde(dive)]
        owner: ArtifactOwner,
        #[garde(custom(digest))]
        sha256: String,
        #[garde(custom(demi_command_protocol::target))]
        target: String,
    },
    /// Asks the backend to revoke this device, as `run uninstall` does;
    /// `revoked` answers, as nothing refuses a revocation.
    Revoke {},
    Hello {
        protocol: u32,
        /// Absent on an unclaimed first start.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[garde(skip)]
        device_token: Option<DeviceToken>,
        #[garde(dive)]
        runner: RunnerInfo,
        /// A number this start of the runner drew, the same for each of its
        /// connections, so the backend tells a runner that kept its jobs
        /// from one that started anew (`crates/runner/src/connection/kept.rs`).
        instance: u64,
        /// The jobs the runner keeps, running or ended and not yet
        /// released.
        #[garde(dive)]
        jobs: Vec<KeptJob>,
        /// Where the session the runner had carries on from; absent when it
        /// had none.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[garde(skip)]
        resume: Option<Resume>,
    },
    /// Liveness, with the count of running jobs the idle rule reads.
    Pong {
        jobs: u64,
    },
    /// The executable of the runner release a `prefetch` named is in
    /// place in the installation.
    Prefetched {
        #[garde(custom(digest))]
        release: String,
    },
    /// A `prefetch` failed, for `reason`; the update after the switch
    /// downloads what is missing.
    PrefetchFailed {
        #[garde(custom(digest))]
        release: String,
        reason: String,
    },
    SyncDone {
        id: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        error: Option<String>,
    },
    FsOk(FsOk),
    /// A failed fs call; `code` is the errno-style code when there is one,
    /// and `readAt` the Host's number as the runner received the call.
    FsError {
        id: String,
        #[garde(dive)]
        read_at: demi_shared_types::ReadAt,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        code: Option<String>,
        message: String,
    },
    GitOk(#[garde(dive)] GitOk),
    /// A failed working-tree call.
    GitError {
        id: String,
        #[garde(dive)]
        read_at: demi_shared_types::ReadAt,
        code: String,
        message: String,
    },
    SpawnOutput {
        spawn_id: String,
        stream: OutputStream,
        bytes: WireBytes,
    },
    /// A raw process's end: the code it exited with; else the name of the
    /// signal that ended it (`SIGKILL`), never other text; else, in
    /// `spawn_error`, why the runner has no status for it.
    SpawnExit {
        spawn_id: String,
        #[serde(deserialize_with = "Option::deserialize")]
        exit_code: Option<i32>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        signal: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        spawn_error: Option<SpawnError>,
    },
    /// A stream's bytes from `offset` while the job runs (`crates/runner-jobs/src/kept_output.rs`). Beyond the stream's first `JOB_VIEW_BYTES`, an
    /// offset past the end of the stream's previous bytes says the runner
    /// left the bytes between out, and a message without bytes says only
    /// that the stream is `offset` bytes long.
    JobOutput {
        job_id: String,
        stream: OutputStream,
        offset: u64,
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
    /// The background tasks that keep a job running once its script has
    /// ended, each by its command line, cut to `JOB_TASK_LINE_BYTES`
    /// (`crates/agent-tools/src/result.rs`): sent when the script ends
    /// while tasks still run, and again each time one of them ends while
    /// others run on. The job's exit follows the last.
    JobOutliving {
        job_id: String,
        #[garde(
            length(min = 1, max = JOB_OUTLIVING_TASKS),
            inner(length(min = 1, max = JOB_TASK_LINE_BYTES))
        )]
        tasks: Vec<String>,
    },
    /// A job's execution context ended: its `demi` commands reach the backend
    /// no more, whether run by the job's own script or by what the script left
    /// running. Sent after the job's `job_exit`, at once when nothing the job
    /// left running holds the context, else when the last of that has ended.
    /// Only a job started with declared commands has a context.
    JobContextEnded { job_id: String },
    /// A job's end, its status given as `spawn_exit` gives a process's.
    JobExit {
        job_id: String,
        #[serde(deserialize_with = "Option::deserialize")]
        exit_code: Option<i32>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        signal: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        spawn_error: Option<SpawnError>,
        /// Absent when bash never ran the script.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        output: Option<OutputLengths>,
        #[garde(length(max = demi_command_protocol::EDIT_JOB_FILES), dive)]
        files: Vec<JobFileChange>,
        /// The renames and removals of the job's embedded utilities, in
        /// order (`crates/agent-store/src/changes.rs`).
        #[garde(length(max = demi_command_protocol::EDIT_JOB_PATH_CHANGES), dive)]
        path_changes: Vec<JobPathChange>,
        files_truncated: bool,
    },
    /// A medium `demi file view` handed to the job, once the job keeps it
    /// (`crates/runner-jobs/src/job_media.rs`). Every one of a job precedes its
    /// `job_exit`.
    JobMedium {
        job_id: String,
        #[garde(range(min = 1))]
        number: u32,
        media_type: String,
        #[garde(range(max = demi_command_protocol::MAX_MEDIUM_BYTES))]
        size: u64,
        #[garde(custom(digest))]
        sha256: String,
        /// Its file's name, as its medium record gave it; a runner of an
        /// earlier release gives only a document's.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        name: Option<String>,
        /// An image's or a video's size in pixels, and a video's length,
        /// as its medium record gave them.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        width: Option<u32>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        height: Option<u32>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        duration_ms: Option<u64>,
    },
    /// The answer to `job_media_read`: the media read flow through the
    /// pipe; each that is not says why.
    JobMediaRead {
        id: String,
        /// Each medium the request named, in its order.
        media: Vec<FileRead>,
    },
    /// The answer to `job_read`: the kept output flows through the pipe; an
    /// error says why none does.
    JobRead {
        id: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        error: Option<String>,
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
    /// This runner's end of a pipe closed: `ok` when its HTTP exchange
    /// completed or, from a sink, when it stopped reading on purpose before
    /// the end, as a job that ended or closed its standard input does, which
    /// ends the pipe for its writer as a whole one; otherwise why it failed.
    PipeDone {
        pipe_id: String,
        ok: bool,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        error: Option<String>,
    },
    /// This runner's upload into a pipe waits in its queue
    /// (`crates/runner-process/src/pipes/uploads.rs`): the pipe waits for its
    /// source without its arrival window until the upload starts.
    PipeQueued {
        pipe_id: String,
    },
    /// Subscription `id` is watched: a change at a number above `since` is
    /// reported, numbers of `epoch` (`crates/runner-host/src/numbers.rs`).
    FsLive {
        id: String,
        since: u64,
        #[garde(length(min = 1))]
        epoch: String,
    },
    /// The subscriptions whose subjects changed in the last 100 ms, each
    /// once, with the number of the latest event that changed it.
    FsChanged {
        #[garde(length(min = 1))]
        changes: Vec<(String, u64)>,
    },
    /// Subscription `id` cannot be watched, for this reason; it is over.
    FsFailed {
        id: String,
        reason: String,
    },
    ResidentOpened {
        stream_id: String,
    },
    ResidentError {
        stream_id: String,
        code: ResidentErrorCode,
        message: String,
    },
    /// A user stream's invocation completed: its exit code and the bounded
    /// tail of its standard error.
    ResidentDone {
        stream_id: String,
        exit_code: u8,
        #[garde(length(chars, max = RESIDENT_STDERR_CHARS))]
        stderr: String,
    },
    /// Lines of the Host's log in `seq` order, on the lowest-priority queue.
    /// `missed` counts the lines the Host's files no longer held since the
    /// last frame.
    LogLines {
        lines: Vec<HostLogLine>,
        missed: u64,
    },
    /// The runner's answer to a `direct_offer`, with its candidates.
    DirectAnswer {
        id: String,
        #[garde(length(min = 1))]
        sdp: String,
    },
    /// A candidate of peer `peer`'s that the runner found after its
    /// answer, such as the address a STUN server saw it at, for the page.
    DirectCandidate {
        #[garde(length(min = 1))]
        peer: String,
        #[garde(length(min = 1, max = crate::direct::CANDIDATE_CHARS))]
        candidate: String,
    },
    /// The answer to a `direct_ping`.
    DirectPong {
        #[garde(length(min = 1))]
        peer: String,
        #[garde(skip)]
        id: u32,
    },
    /// The runner did not answer a `direct_offer`.
    DirectRefused {
        id: String,
        code: crate::direct::OfferRefusal,
        message: String,
    },
    /// A direct channel's `stream` of `conversation`, named by the id the
    /// runner gave it, of the user stream `name` its header asked for, opened,
    /// or closed (`crates/runner-protocol/src/direct.rs`).
    ///
    /// The backend knows each open one as a stream of that name it opened,
    /// counts it as the conversation's activity and answers its artifact
    /// requests.
    DirectStream {
        stream: String,
        name: String,
        #[garde(custom(conversation_name))]
        conversation: String,
        open: bool,
    },
    /// Whether anyone watches the open direct stream `stream` now, as its
    /// page said (`crates/runner-protocol/src/direct.rs`): only a watched one
    /// is the conversation's activity.
    DirectStreamWatched {
        stream: String,
        watched: bool,
    },
    /// Asks the backend to introduce this runner to the runner of `device`,
    /// the other end of a forward, as peer `peer`, with this runner's offer;
    /// `direct_dialed` or `direct_dial_refused` answers.
    DirectDial {
        id: String,
        #[garde(length(min = 1))]
        peer: String,
        #[garde(length(min = 1))]
        device: String,
        #[garde(length(min = 1))]
        sdp: String,
    },
    /// Peer `peer`, which the backend introduced to another runner, ended:
    /// the other runner closes its side.
    DirectClosed {
        #[garde(length(min = 1))]
        peer: String,
    },
    /// What forward `forward` is on this runner, its Listen On, now: none
    /// once it no longer listens.
    ForwardState {
        #[garde(length(min = 1))]
        forward: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        state: Option<crate::forward::ForwardRunState>,
    },
    /// The linked worktrees of the repository whose main worktree is the
    /// project directory `directory`, named as the backend's `projects`
    /// named it; none for a directory that is no repository's main worktree.
    /// It goes before any message that could follow a change of them
    /// (`crates/runner-host/src/worktrees.rs`).
    Worktrees {
        #[garde(length(min = 1))]
        directory: String,
        #[garde(dive)]
        worktrees: Vec<LinkedWorktree>,
    },
    /// A connection of forward `forward` that goes through the relay; the
    /// backend answers `forward_opened` or `forward_refused`.
    ForwardOpen {
        id: String,
        #[garde(length(min = 1))]
        forward: String,
    },
    /// The port answered the backend's `http_request` `id`: its status and
    /// headers; the body follows in the request's answer pipe.
    HttpAnswered {
        id: String,
        #[garde(range(min = 100, max = 999))]
        status: u16,
        #[garde(length(max = crate::exposure::MAX_HEADERS), dive)]
        headers: Vec<crate::exposure::HttpHeader>,
    },
    /// The request `id` got no answer: the port refused the connection, or
    /// the request could not be made, as `message` says.
    HttpFailed {
        id: String,
        message: String,
    },
    /// Every artifact this runner's cache holds, once the connection is
    /// online and whenever an install or a retirement changes them
    /// (`crates/runner-command-packages/src/cache.rs`).
    Installed {
        #[garde(dive)]
        artifacts: Vec<HostArtifact>,
    },
    /// The artifacts this runner's cache is downloading and how far each
    /// came, once the connection is online and as they change, at most four
    /// times a second (`crates/runner-command-packages/src/cache.rs`).
    Downloading {
        #[garde(dive)]
        downloads: Vec<HostDownload>,
    },
}

/// One artifact a Host holds: its package, its line's name and its
/// version, such as `demi.browser`, `Chrome for Testing` and
/// `153.0.8010.36`.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, garde::Validate, JsonSchema,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostArtifact {
    #[garde(length(min = 1, max = 200))]
    pub package: String,
    #[garde(length(min = 1, max = 100))]
    pub name: String,
    #[garde(length(min = 1, max = 100))]
    pub version: String,
}

/// An artifact a Host is downloading into its cache: its package, line and
/// version, as a [`HostArtifact`] names them, and how many of its `size`
/// bytes arrived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostDownload {
    #[garde(length(min = 1, max = 200))]
    pub package: String,
    #[garde(length(min = 1, max = 100))]
    pub name: String,
    #[garde(length(min = 1, max = 100))]
    pub version: String,
    #[garde(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
    pub received: u64,
    #[garde(range(min = 1, max = demi_shared_types::MAX_SAFE_INTEGER))]
    pub size: u64,
}

/// Why a hello was refused. `already_connected` is the one outcome a runner
/// retries: the token's live connection may be a half-open socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelloErrorCode {
    UnsupportedProtocol,
    UnknownDevice,
    AlreadyConnected,
    Internal,
}

/// A signal a kill request names (`crates/runner-host/src/lib.rs`): one
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// Why a `resident_open` stream never opened: the package lacks the operation,
/// its resident could not start, or the runner turned the stream away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidentErrorCode {
    UnknownOperation,
    ResidentFailed,
    Refused,
}

/// A file the backend offers a runner for download: the origin-relative URL
/// the runner `GET`s it from, with its device token and Range requests, and
/// the size and SHA-256 of its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileOffer {
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// One end of a pipe: the id the runner reports `pipe_done` under, and the
/// origin-relative URL its end `PUT`s to or `GET`s from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipeRef {
    pub id: String,
    pub url: String,
    /// Whether a page waits to show what the runner uploads into the pipe,
    /// which then goes ahead of the uploads a command or the agent waits for
    /// (`crates/runner-process/src/pipes/uploads.rs`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shown: bool,
}

/// What authorizes an artifact: a job and the manifest it runs with, a user
/// stream, or the package programs the backend sent the runner to keep, of
/// its own release or, with `next`, of the release a prefetch named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(untagged)]
pub enum ArtifactOwner {
    Job(#[garde(dive)] JobArtifactOwner),
    Stream(#[garde(skip)] StreamArtifactOwner),
    Programs(#[garde(skip)] ProgramsArtifactOwner),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramsArtifactOwner {
    pub next: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobArtifactOwner {
    #[garde(skip)]
    pub job_id: String,
    #[garde(custom(digest))]
    pub manifest_hash: String,
}

/// A pipe a spawned process inherits at descriptor `fd`, which holds `bytes`
/// and then ends. Descriptors 0 to 2 are the standard streams.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Descriptor {
    #[garde(range(min = 3))]
    pub fd: u32,
    #[garde(skip)]
    pub bytes: WireBytes,
}

impl std::fmt::Debug for Descriptor {
    /// The bytes may be a secret, so only their length is shown.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Descriptor")
            .field("fd", &self.fd)
            .field("bytes", &format_args!("{} bytes", self.bytes.0.len()))
            .finish()
    }
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
    pub platform: RunnerPlatform,
    /// The Host's operating system, which the agent reads in its context
    /// block (`crates/backend-host-access/src/transition.rs`).
    #[garde(dive)]
    pub os: OperatingSystem,
    /// The runner's release, such as `0.1.16`, which Settings shows.
    #[garde(length(chars, min = 1, max = 64))]
    pub version: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(custom(demi_command_protocol::target)))]
    pub native_target: Option<String>,
    /// Read at shell creation, so it arrives before any Host use.
    pub identity: HostIdentity,
    /// The kernel a process the runner starts runs on, which the Claude
    /// Code CLI describes its machine by
    /// (`crates/provider-claude-code/src/session.rs`); none on Windows, which
    /// has no `uname`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(dive)]
    pub kernel: Option<Kernel>,
    /// The Host's physical memory in bytes, which bounds how many idle Claude
    /// Code processes its machine keeps
    /// (`crates/provider-claude-code/src/idle.rs`).
    pub memory: u64,
    /// The `SHELL` of the device environment, which every process the
    /// runner starts inherits; none when it names none.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(length(chars, min = 1, max = 4096)))]
    pub shell: Option<String>,
    /// A runner booted as a managed host's init: it presents its token or is
    /// refused, never paired.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub managed: Option<bool>,
    /// A paired device's installation directory, whose launcher starts the
    /// runner again (`crates/runner/src/console.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(length(chars, min = 1, max = 4096)))]
    pub installation: Option<String>,
    /// Why the device will not start its runner by itself after a restart,
    /// such as a Linux without a systemd user manager; none when its
    /// registration with the system's service manager is in place
    /// (`crates/runner/src/autostart.rs`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(dive)]
    pub autostart_unavailable: Option<AutostartUnavailable>,
}

/// Why a device will not start its runner by itself after a restart
/// (`crates/runner/src/autostart.rs`): its cause, with what the
/// system said, so that Devices says it in the user's language and the
/// runner's log and installer in English ([`AutostartUnavailable::sentence`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "cause",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AutostartUnavailable {
    /// The system does not run systemd, as most containers and WSL
    /// without it.
    NoSystemd,
    /// systemd's user manager does not answer, as without the user's
    /// session; `error` is what `systemctl` said.
    NoUserManager {
        #[garde(length(chars, max = 4096))]
        error: String,
    },
    /// The file that registers the runner, at `path`, could not be
    /// written; `error` is the system's reason.
    NotWritten {
        #[garde(length(chars, min = 1, max = 4096))]
        path: String,
        #[garde(length(chars, max = 4096))]
        error: String,
    },
    /// Lingering is off for `user` and the system refused to turn it on,
    /// so the runner starts only once the user logs in.
    LingerOff {
        #[garde(length(chars, min = 1, max = 256))]
        user: String,
    },
    /// Windows refused the scheduled task and the `Run` value; `error` is
    /// what it said.
    Refused {
        #[garde(length(chars, max = 4096))]
        error: String,
    },
    /// The installation's directory name, `name`, cannot name a
    /// registration.
    UnusableName {
        #[garde(length(chars, min = 1, max = 256))]
        name: String,
    },
    /// The user database does not name the account the runner runs as.
    UnknownAccount,
}

impl AutostartUnavailable {
    /// The cause as the log names it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoSystemd => "no_systemd",
            Self::NoUserManager { .. } => "no_user_manager",
            Self::NotWritten { .. } => "not_written",
            Self::LingerOff { .. } => "linger_off",
            Self::Refused { .. } => "refused",
            Self::UnusableName { .. } => "unusable_name",
            Self::UnknownAccount => "unknown_account",
        }
    }

    /// The cause in English, as the runner's log and the installers say it
    /// after [`crate::console::NO_AUTOSTART`]; a path under `home` is
    /// written from `~`.
    pub fn sentence(&self, home: &str) -> String {
        match self {
            Self::NoSystemd => "this system does not run systemd".into(),
            Self::NoUserManager { error } => {
                format!("systemd's user manager does not answer ({error})")
            }
            Self::NotWritten { path, error } => format!(
                "the file that registers it, {}, could not be written ({error})",
                from_home(path, home)
            ),
            Self::LingerOff { user } => format!(
                "it starts only once {user} logs in; to start it at boot, run: sudo loginctl enable-linger {user}"
            ),
            Self::Refused { error } => {
                format!("Windows refused its scheduled task and its startup entry ({error})")
            }
            Self::UnusableName { name } => {
                format!("the installation's directory name {name} cannot name a registration")
            }
            Self::UnknownAccount => "the user database does not name this account".into(),
        }
    }
}

/// `path` written from `~` when it lies in `home`.
fn from_home(path: &str, home: &str) -> String {
    let home = home.trim_end_matches(['/', '\\']);
    match path.strip_prefix(home) {
        Some(rest) if !home.is_empty() && (rest.starts_with('/') || rest.starts_with('\\')) => {
            format!("~{rest}")
        }
        _ => path.to_owned(),
    }
}

/// The operating system a runner runs on, named as Node's
/// `process.platform` names it. A device keeps its runner's, and the web app
/// receives it with the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunnerPlatform {
    Darwin,
    Win32,
    Linux,
}

/// A Host's operating system as its runner reports it in its hello and a
/// device keeps it: its name with its release, on Linux its distribution,
/// and the machine's architecture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperatingSystem {
    /// Such as `Ubuntu 26.04`, `macOS 26.5` or `Windows 11 Pro (build
    /// 26100)`.
    #[garde(length(chars, min = 1, max = 256))]
    pub name: String,
    /// On Linux, the distribution's os-release `ID`, such as `ubuntu` or
    /// `fedora`, which picks the logo of the device's mark; none on other
    /// systems.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(inner(length(chars, min = 1, max = 64)))]
    pub distribution: Option<String>,
    /// The processor architecture as Rust names it, such as `x86_64` or
    /// `aarch64`.
    #[garde(length(chars, min = 1, max = 64))]
    pub arch: String,
}

/// A Host's kernel as `uname -s` and `uname -r` print it, read once when
/// the runner starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Kernel {
    /// Such as `Linux` or `Darwin`.
    #[garde(length(chars, min = 1, max = 256))]
    pub name: String,
    /// Such as `6.8.0-48-generic` or `25.5.0`.
    #[garde(length(chars, min = 1, max = 256))]
    pub release: String,
}

/// The account the runner works as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostIdentity {
    pub uid: u32,
    pub gid: u32,
    pub hostname: String,
    pub home_dir: String,
    /// The absolute directory that holds the conversations' scratch
    /// directories, `<DEMI_HOME>/scratch` (`crates/runner-jobs/src/scratch.rs`).
    pub scratch_root: String,
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub uid: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub gid: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub ino: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub dev: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub nlink: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub is_character_device: Option<bool>,
    #[serde(
        rename = "isFIFO",
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub is_fifo: Option<bool>,
}

/// A file an `fs_readFile` opened: its metadata then, its version, and
/// whether that is the version the request named, in which case nothing
/// streams (`crates/runner-host/src/files.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenedFile {
    pub stat: FileStat,
    pub version: String,
    pub unchanged: bool,
}

/// What `fs_hashFile` answers: the bytes hashed, their SHA-256, which names
/// their blob, and the full path of the file read, its symbolic links and
/// `..` resolved by the Host's file system as that Host writes it; or, for a
/// file over the request's limit, its size alone (`too_large`), none of its
/// bytes read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum FileHash {
    Hashed {
        size: u64,
        sha256: BlobRef,
        path: String,
    },
    TooLarge { size: u64 },
}

impl FileHash {
    /// The answer for `size` bytes of the file at `path` whose SHA-256 is
    /// `sha256`, in lowercase hexadecimal; anything else is refused.
    pub fn hashed(size: u64, sha256: String, path: String) -> Result<Self, String> {
        let sha256 = BlobRef::try_from(sha256)?;
        Ok(Self::Hashed { size, sha256, path })
    }
}

/// A file's version, from its size and modification time in milliseconds:
/// a weak ETag, the one the web app's file routes answer. A negative time is
/// written with a minus sign, as JavaScript's `toString(16)` writes it.
pub fn file_version(size: u64, modified_ms: i64) -> String {
    let time = if modified_ms < 0 {
        format!("-{:x}", modified_ms.unsigned_abs())
    } else {
        format!("{modified_ms:x}")
    };
    format!("W/\"{size:x}-{time}\"")
}

/// One entry of a directory listing, with its type, size and modification
/// time as the entry's own metadata reports them, as `lstat` does: a
/// symbolic link's are the link's, and it is neither a file nor a
/// directory. A symbolic link, and on Windows a junction, carries `link`;
/// no other entry does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DirEntry {
    pub name: String,
    pub is_file: bool,
    pub is_directory: bool,
    pub size: u64,
    pub mtime: Timestamp,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub link: Option<SymbolicLink>,
}

/// Where a symbolic link of a listing points (`crates/runner-host/src/lib.rs`): its target as written, and what is there once every link
/// on the way is followed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SymbolicLink {
    pub target: String,
    pub leads_to: LinkTarget,
}

/// What a symbolic link leads to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LinkTarget {
    File,
    Directory,
    /// Neither a file nor a directory, such as a device or a pipe.
    Other,
    /// Nothing is there: a name on the way does not exist, or is no
    /// directory.
    Missing,
    /// Something may be there but cannot be reached: a directory on the way
    /// may not be searched, the links go round in a loop, the disk fails.
    /// `reason` is the system's, as `strerror` words it.
    Unreadable { reason: String },
}

/// One file of a directory an `fs_writeDirectory` writes: its path inside
/// the directory and its mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DirectoryFile {
    pub path: String,
    pub mode: u32,
}

/// What a write does when its path is taken (`crates/runner-host/src/lib.rs`). A directory at the path is never written over: the write
/// fails with `EISDIR`, unless it takes another name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteExists {
    /// Replace the file.
    Replace,
    /// Fail with `EEXIST`.
    Refuse,
    /// Take the first free name of the form `name-2.ext`, `name-3.ext`; a
    /// file of the same bytes under one of those names, the given one
    /// first, is the file, and nothing is written.
    Rename,
}

/// Whether one file of a read of several was read. The read pipe carries
/// each read file as its length, eight bytes in big-endian order, then that
/// many bytes; a file not read has nothing in the pipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum FileRead {
    Read,
    /// Why it was not read: `code` names the cause as an fs error does,
    /// `too_large` for a file over the request's limit.
    Failed {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        code: Option<String>,
        message: String,
    },
}

/// One path an `fs_look` names, and the most bytes of a file there to read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LookPath {
    pub path: String,
    pub limit: u64,
}

/// What an `fs_look` found at one path. A symbolic link at the path is
/// followed; a directory's entries name a link as a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Looked {
    /// Nothing is there, or a file stands where a directory of the path
    /// should be.
    Missing,
    Directory {
        entries: Vec<DirEntry>,
    },
    /// A file of `size` bytes, whose first `read` bytes are the next in the
    /// look's output.
    File {
        size: u64,
        read: u64,
    },
    /// Neither a file nor a directory.
    Other,
    /// The runner could not read it, such as for want of permission.
    Unreadable {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        code: Option<String>,
        message: String,
    },
}

/// A working tree's changes, as `git status` lists them. The web app
/// receives it too, beside the directory it lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct GitChanges {
    pub repository: bool,
    /// The absolute path of the repository's git directory; null outside a
    /// repository. Named as the web API names its fields, since the page
    /// receives it as it is.
    #[serde(rename = "gitDir", deserialize_with = "Option::deserialize")]
    #[schemars(with = "demi_shared_types::Nullable<String>")]
    pub git_dir: Option<String>,
    /// Always written, null before the first commit.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "demi_shared_types::Nullable<String>")]
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    pub from: Option<String>,
    #[garde(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
    pub added: u64,
    #[garde(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
    pub removed: u64,
}

/// A repository's local branches (`crates/runner-host/src/git.rs`): by name,
/// most recently committed first; the one checked out, or null with `head`
/// when HEAD is detached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct GitBranches {
    pub branches: Vec<String>,
    /// Null while HEAD is detached.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "demi_shared_types::Nullable<String>")]
    pub current: Option<String>,
    /// The commit a detached HEAD names; null while a branch is checked out.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "demi_shared_types::Nullable<String>")]
    pub head: Option<String>,
}

/// One commit as a history lists it: its full id, its whole message, its
/// author's name and its commit time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct GitLogEntry {
    pub id: String,
    pub message: String,
    pub author: String,
    pub time: demi_shared_types::Timestamp,
}

/// A commit's time from git's seconds since the Unix epoch; a time outside
/// what a timestamp holds is no time anyone committed at, and reads as the
/// epoch.
pub fn commit_time(seconds: i64) -> demi_shared_types::Timestamp {
    demi_shared_types::Timestamp::from_millisecond(seconds.saturating_mul(1000))
        .unwrap_or(demi_shared_types::Timestamp::UNIX_EPOCH)
}

/// One page of a first-parent history, newest first; `more` when older
/// commits follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct GitLog {
    #[garde(dive)]
    pub commits: Vec<GitLogEntry>,
    pub more: bool,
}

/// A commit as `git_commit` answers it: its entry with its parents, its
/// files against its first parent, cut at 5,000 (`truncated`), and the local
/// branches it is on, the checked-out one first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct GitCommit {
    #[garde(dive)]
    pub commit: GitCommitEntry,
    #[garde(dive)]
    pub files: Vec<GitCommitFile>,
    pub truncated: bool,
    pub branches: Vec<String>,
}

/// A [`GitLogEntry`] with the commit's parents, the first first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct GitCommitEntry {
    pub id: String,
    pub message: String,
    pub author: String,
    pub time: demi_shared_types::Timestamp,
    pub parents: Vec<String>,
}

/// One file a commit changed, against its first parent, or added by a root
/// commit: how, where a renamed file came from, and the lines added and
/// removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct GitCommitFile {
    pub path: String,
    /// The letter git's log gives the file: `A`, `M`, `D` or `R`.
    #[garde(pattern(r"^[AMDR]$"))]
    pub status: String,
    pub kind: ChangeKind,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    pub from: Option<String>,
    #[garde(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
    pub added: u64,
    #[garde(range(max = demi_shared_types::MAX_SAFE_INTEGER))]
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

/// Why the runner has no status for a process or job: it could not start, or
/// the runner failed it before its end was known, which is `Other`. `detail`
/// holds the runner's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnError {
    pub kind: SpawnErrorKind,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
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

/// Each stream's length when a job ended. The backend reads what it did not
/// receive of them from the job's kept output (`crates/runner-jobs/src/kept_output.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputLengths {
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
}

/// A job a runner keeps through a connection loss, as its hello lists it
/// (`crates/runner/src/connection/kept.rs`): its end once it ended, each stream's
/// length, how many media it keeps, and the tasks that outlive its script. The exit of a job that ended follows
/// the hello's answer, as when the job ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct KeptJob {
    #[garde(length(min = 1, max = 64))]
    pub job_id: String,
    /// Its end; none while it runs.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub ended: Option<KeptEnd>,
    pub output: OutputLengths,
    pub media: u32,
    /// While it runs, the background tasks that keep it running once its
    /// script has ended, as its last `job_outliving` named them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(
        length(max = JOB_OUTLIVING_TASKS),
        inner(length(min = 1, max = JOB_TASK_LINE_BYTES))
    )]
    pub outliving: Vec<String>,
}

/// A linked worktree of a project's repository, as git keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinkedWorktree {
    /// Its name: its directory's under the repository's `.git/worktrees/`.
    #[garde(length(min = 1))]
    pub name: String,
    /// Its directory, canonical where it exists.
    #[garde(length(min = 1))]
    pub path: String,
}

/// How a kept job ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeptEnd {
    #[serde(deserialize_with = "Option::deserialize")]
    pub exit_code: Option<i32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub signal: Option<String>,
}

/// A file a job changed: its edit record entry and its line counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct JobFileChange {
    #[garde(length(min = 1), custom(without_nul))]
    pub path: String,
    #[garde(skip)]
    pub kind: EditKind,
    #[garde(length(max = demi_command_protocol::EDIT_JOB_SEGMENTS as usize), dive)]
    pub edits: Vec<EditCopies>,
    #[garde(skip)]
    pub added: u64,
    #[garde(skip)]
    pub removed: u64,
}

/// A rename or a removal of a job's embedded utilities, as its edit record
/// names it (`crates/command-protocol/src/edits.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum JobPathChange {
    Renamed {
        #[garde(length(min = 1), custom(without_nul))]
        from: String,
        #[garde(length(min = 1), custom(without_nul))]
        to: String,
    },
    Removed {
        #[garde(length(min = 1), custom(without_nul))]
        path: String,
    },
}

// Closed sets display as the wire spells them, for logs.
serde_plain::derive_display_from_serialize!(HelloErrorCode);
serde_plain::derive_display_from_serialize!(Signal);
serde_plain::derive_display_from_serialize!(OutputStream);
serde_plain::derive_display_from_serialize!(ResidentErrorCode);
serde_plain::derive_display_from_serialize!(ChangeKind);
serde_plain::derive_display_from_serialize!(SpawnErrorKind);
serde_plain::derive_display_from_serialize!(RunnerPlatform);
// A device's platform is stored as its name.
serde_plain::derive_fromstr_from_deserialize!(RunnerPlatform);

impl Outbound {
    /// How the message travels the link's session
    /// (`crates/runner-protocol/src/wire/session.rs`): a message of the
    /// connection's own, one that goes on across a drop, such as a job's output
    /// and exit or a call's, or an answer or event of the connection it was
    /// written on, such as a Host request's reply, which the backend no longer
    /// waits for.
    pub fn carriage(&self) -> Carriage {
        // Every message is named, so a new one is classified when it is added.
        match self {
            Self::Hello { .. }
            | Self::Pong { .. } => Carriage::Connection,
            Self::Revoke { .. }
            | Self::Installed { .. }
            | Self::Downloading { .. }
            | Self::PackageMessage { .. }
            | Self::PackageLost { .. }
            | Self::JobOutput { .. }
            | Self::JobRunningHint { .. }
            | Self::JobOutliving { .. }
            | Self::JobContextEnded { .. }
            | Self::JobExit { .. }
            | Self::JobMedium { .. }
            | Self::RpcCall { .. }
            | Self::RpcStdin { .. }
            | Self::RpcStdinEnd { .. }
            | Self::RpcCancel { .. }
            | Self::PipeDone { .. }
            | Self::PipeQueued { .. }
            | Self::Worktrees { .. } => Carriage::Outlasting,
            Self::ConversationReleased { .. }
            | Self::PackageReleased { .. }
            | Self::NumbersReserve { .. }
            | Self::ArtifactResolve { .. }
            | Self::Prefetched { .. }
            | Self::PrefetchFailed { .. }
            | Self::SyncDone { .. }
            | Self::FsOk { .. }
            | Self::FsError { .. }
            | Self::GitOk { .. }
            | Self::GitError { .. }
            | Self::SpawnOutput { .. }
            | Self::SpawnExit { .. }
            | Self::JobMediaRead { .. }
            | Self::JobRead { .. }
            | Self::FsLive { .. }
            | Self::FsChanged { .. }
            | Self::FsFailed { .. }
            | Self::ResidentOpened { .. }
            | Self::ResidentError { .. }
            | Self::ResidentDone { .. }
            | Self::LogLines { .. }
            | Self::DirectAnswer { .. }
            | Self::DirectCandidate { .. }
            | Self::DirectPong { .. }
            | Self::DirectRefused { .. }
            | Self::DirectStream { .. }
            | Self::DirectStreamWatched { .. }
            | Self::DirectDial { .. }
            | Self::DirectClosed { .. }
            | Self::ForwardState { .. }
            | Self::ForwardOpen { .. }
            | Self::HttpAnswered { .. }
            | Self::HttpFailed { .. } => Carriage::Scoped,
        }
    }
}
