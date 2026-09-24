//! `RemoteHost`: a Host over its device's runner connection
//! (`runner.md` § Host operations). Every filesystem operation is one request
//! and its reply, with a file's contents in a pipe beside it; processes and
//! jobs stream over their own messages. A Host is a value: two handles with
//! equal keys are the same Host, whichever connection serves them, and a
//! handle made while its runner is away serves once it is back.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use bytes::{Bytes, BytesMut};
use demi_command_service::protocol::{CommandContext, PackageDescriptor};
use demi_core::Timestamp;
use demi_gates::GateLease;
use demi_runner_protocol::wire::{
    self, FsResult, GitChanges, GitResult, Inbound, LogLine, PipeRef, Readdir, STDIN_CHUNK_BYTES,
    WireBytes,
};
use demi_shell::{
    ByteRange, ByteStream, CpOptions, DirEntry, FileContents, FileKind, FileStat, Host, HostError,
    HostErrorKind, HostFs, HostIdentity, HostKey, HostProcess, JobCaller, MkdirOptions, Process,
    ProcessControl, ProcessEnd, ProcessOutput, RmOptions, Signal, SpawnEnv, SpawnRequest,
    WriteOptions,
};
use futures_util::{StreamExt, future::LocalBoxFuture};
use serde_json::{Map, Value};
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    ArtifactResolver, Link,
    link::{Answer, Expected, JobEnd, JobEntry, JobOrigin, ServiceEntry, Shared, SpawnEntry},
    manifest::CommandSelection,
    pipes::{Pipe, PipeError, PipeReader},
};

/// A device's runner connection as its Hosts see it.
#[derive(Clone)]
pub enum DeviceLink {
    Online(Link),
    /// No runner is connected; `last` is the account it reported when it
    /// last was.
    Offline {
        last: Option<HostIdentity>,
    },
}

/// Whether a Host's operations may start: all of them on a paired device;
/// on a Cloud, each holds a lease of the Cloud's admission, which refuses
/// while the Cloud changes state.
#[derive(Clone)]
pub enum Admission {
    Free,
    Leased(Rc<dyn Fn() -> Result<GateLease, HostError>>),
}

/// A Host over a runner connection.
#[derive(Clone)]
pub struct RemoteHost(Rc<Inner>);

struct Inner {
    key: HostKey,
    default_cwd: String,
    link: watch::Receiver<DeviceLink>,
    admission: Admission,
}

impl RemoteHost {
    pub fn new(
        key: HostKey,
        default_cwd: String,
        link: watch::Receiver<DeviceLink>,
        admission: Admission,
    ) -> Self {
        Self(Rc::new(Inner {
            key,
            default_cwd,
            link,
            admission,
        }))
    }

    /// Whether the device's runner is connected.
    pub fn online(&self) -> bool {
        self.link().is_ok()
    }

    fn link(&self) -> Result<Link, HostError> {
        match &*self.0.link.borrow() {
            DeviceLink::Online(link) if !link.is_closed() => Ok(link.clone()),
            DeviceLink::Online(link) => Err(link.offline()),
            DeviceLink::Offline { .. } => Err(HostError::offline("runner disconnected")),
        }
    }

    fn admit(&self) -> Result<Option<GateLease>, HostError> {
        match &self.0.admission {
            Admission::Free => Ok(None),
            Admission::Leased(admit) => admit().map(Some),
        }
    }

    fn cwd(&self) -> Option<String> {
        Some(self.0.default_cwd.clone())
    }

    /// Runs `script` as one shell job. Offline, the job has ended already;
    /// a Host that admits no work refuses it.
    pub async fn start_job(&self, job: JobStart) -> Result<RemoteJob, HostError> {
        let lease = self.admit()?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let shared = Shared::new();
        let Ok(link) = self.link() else {
            shared.finish(JobEnd::lost("runner disconnected"));
            return Ok(RemoteJob {
                id,
                shared,
                link: None,
            });
        };
        let origin = Rc::new(JobOrigin {
            host: self.0.key.clone(),
            context: job.context.clone(),
            caller: job.caller.clone(),
        });
        link.with_state(|state| {
            state.add_job(
                id.clone(),
                JobEntry {
                    shared: shared.clone(),
                    origin,
                    commands: job.commands.clone(),
                    cancel: CancellationToken::new(),
                    _lease: lease,
                },
            );
        });
        // A job's manifest and its start travel together: the runner holds
        // one manifest at a time.
        let _turn = link.job_turn().acquire().await;
        let sent = async {
            if let Some(commands) = &job.commands
                && link.sent_manifest().as_deref() != Some(commands.hash())
            {
                link.set_sent_manifest(Some(commands.hash().to_owned()));
                link.send(&Inbound::Manifest {
                    manifest: commands.wire().clone(),
                })
                .await?;
            }
            link.send(&Inbound::JobStart {
                job_id: id.clone(),
                manifest_hash: job
                    .commands
                    .as_ref()
                    .map(|commands| commands.hash().to_owned()),
                context: job.context,
                script: job.script,
                cwd: job.cwd,
                env: job.env,
                stdin: job.stdin,
                stdout: job.stdout,
            })
            .await
        };
        if let Err(error) = sent.await {
            // What the runner received is uncertain: the next job sends its
            // manifest again.
            link.set_sent_manifest(None);
            link.with_state(|state| state.remove_job(&id));
            shared.finish(JobEnd::lost(&error.message));
            return Err(error);
        }
        Ok(RemoteJob {
            id,
            shared,
            link: Some(link),
        })
    }

    /// A pipe the device's runner fills with `range` of the file, once the
    /// file is open: a file that cannot be opened fails here, before any
    /// byte. Dropping the reader stops the read.
    pub async fn read_pipe(&self, path: &str, range: ByteRange) -> Result<PipeReader, HostError> {
        let link = self.link()?;
        let _lease = self.admit()?;
        filled(&link, Expected::Fs("readFile"), |id, output| {
            Inbound::FsReadFile {
                id,
                path: path.into(),
                cwd: self.cwd(),
                offset: (range.offset > 0).then_some(range.offset),
                length: range.length,
                output,
            }
        })
        .await
    }

    /// A pipe to the device's runner that this process fills.
    pub fn write_pipe(&self) -> Result<Pipe, HostError> {
        let link = self.link()?;
        Ok(link.pipes().to_device(link.device()))
    }

    /// Writes the file from `input`, a pipe from [`RemoteHost::write_pipe`]:
    /// the runner puts the file in place once it read the pipe to its end,
    /// and then replies. A pipe that fails leaves the file as it was.
    pub async fn write_from(
        &self,
        path: &str,
        input: &Pipe,
        options: WriteOptions,
    ) -> Result<(), HostError> {
        let link = self.link()?;
        let _lease = self.admit()?;
        link.call(Expected::Fs("writeFile"), |id| Inbound::FsWriteFile {
            id,
            path: path.into(),
            cwd: self.cwd(),
            create_parents: options.create_parents.then_some(true),
            input: input.wire_ref(),
        })
        .await
        .map(|_| ())
    }

    /// The uncommitted changes under `root` (`runner.md` § Working tree).
    pub async fn git_changes(&self, root: &str) -> Result<GitChanges, HostError> {
        let link = self.link()?;
        let reply = link
            .call(Expected::Git("changes"), |id| Inbound::GitChanges {
                id,
                root: root.into(),
            })
            .await?;
        match reply {
            Answer::Git(GitResult::Changes(changes)) => Ok(changes),
            _ => Err(protocol(
                "the runner answered another working-tree operation",
            )),
        }
    }

    /// `path`, relative to `root`, as the last commit has it.
    pub async fn git_show(&self, root: &str, path: &str) -> Result<Bytes, HostError> {
        let link = self.link()?;
        let reader = filled(&link, Expected::Git("show"), |id, output| {
            Inbound::GitShow {
                id,
                root: root.into(),
                path: path.into(),
                output,
            }
        })
        .await?;
        collect(reader).await
    }

    /// Up to `limit` lines of the Host's log after `since`, oldest first, of
    /// one `source` when named (`runner.md` § Host log).
    pub async fn read_log(
        &self,
        since: Option<u64>,
        limit: u64,
        source: Option<&str>,
    ) -> Result<LogPage, HostError> {
        let link = self.link()?;
        let reply = link
            .call(Expected::Log, |id| Inbound::LogRead {
                id,
                since,
                limit,
                source: source.map(str::to_owned),
            })
            .await?;
        match reply {
            Answer::Log { lines, next } => Ok(LogPage { lines, next }),
            _ => Err(protocol("the runner answered another request")),
        }
    }

    /// Opens a TCP stream on the device's network between the socket and the
    /// two pipes (`runner.md` § Network streams). It resolves when the runner
    /// connected; a refusal carries its code.
    pub async fn open_net(
        &self,
        host: &str,
        port: u16,
        input: PipeRef,
        output: PipeRef,
    ) -> Result<(), HostError> {
        let link = self.link()?;
        let _lease = self.admit()?;
        let stream_id = uuid::Uuid::new_v4().simple().to_string();
        let opened = link.wait_for(stream_id.clone(), Expected::Net)?;
        link.send(&Inbound::NetOpen {
            stream_id,
            host: host.into(),
            port,
            input,
            output,
        })
        .await?;
        opened.receive().await.map(|_| ())
    }

    /// Opens a user stream: `request`'s operation in the resident service
    /// that holds the conversation's state, its input and output the two
    /// pipes (`runner.md` § Service streams). It admits no work: a stream is
    /// retention, not activity.
    pub async fn open_service(
        &self,
        request: ServiceRequest,
        input: PipeRef,
        output: PipeRef,
    ) -> Result<ServiceStream, HostError> {
        let link = self.link()?;
        let stream_id = uuid::Uuid::new_v4().simple().to_string();
        let (done, ended) = oneshot::channel();
        link.with_state(|state| {
            state.add_service(
                stream_id.clone(),
                ServiceEntry {
                    package: request.package.clone(),
                    resolver: request.resolver.clone(),
                    cancel: CancellationToken::new(),
                    done: Some(done),
                },
            );
        });
        let stream = ServiceStream {
            link: link.clone(),
            stream_id: stream_id.clone(),
            ended,
        };
        let opened = link.wait_for(stream_id.clone(), Expected::Service)?;
        link.send(&Inbound::ServiceOpen {
            stream_id,
            context: request.context,
            package: request.package,
            operation: request.operation,
            args: request.args,
            json: request.json,
            cwd: request.cwd,
            input,
            output,
        })
        .await?;
        opened.receive().await?;
        Ok(stream)
    }

    /// One question, one answer: opens a service stream, sends `input` as its
    /// whole input, and returns its whole output, at most `max_bytes` of it.
    /// An invocation that exits nonzero fails with its code, its standard
    /// error's tail and its output.
    pub async fn call_service(
        &self,
        request: ServiceRequest,
        input: Bytes,
        max_bytes: usize,
    ) -> Result<Bytes, ServiceCallError> {
        let link = self.link()?;
        let to_service = link.pipes().to_device(link.device());
        let from_service = link.pipes().from_device(link.device());
        let mut writer = to_service.writer().map_err(pipe_error)?;
        let mut reader = from_service.reader().map_err(pipe_error)?;
        let abandon = |reason: &str| {
            to_service.fail(reason);
            from_service.fail(reason);
        };
        let mut stream = match self
            .open_service(request, to_service.wire_ref(), from_service.wire_ref())
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                abandon("service call failed");
                return Err(error.into());
            }
        };
        let sent = writer.write(input).await;
        if let Err(failure) = sent {
            abandon("service call failed");
            return Err(HostError::interrupted(failure.to_string()).into());
        }
        writer.end();
        let mut answer = BytesMut::new();
        while let Some(chunk) = reader.next().await {
            let chunk = chunk.map_err(|failure| HostError::interrupted(failure.to_string()))?;
            if answer.len() + chunk.len() > max_bytes {
                abandon("service call failed");
                return Err(ServiceCallError::TooLarge(max_bytes));
            }
            answer.extend_from_slice(&chunk);
        }
        let end = stream.done().await?;
        let answer = answer.freeze();
        if end.exit_code != 0 {
            return Err(ServiceCallError::Exited {
                exit_code: end.exit_code,
                stderr: end.stderr,
                stdout: answer,
            });
        }
        Ok(answer)
    }

    /// Asks the runner to release what its services hold for
    /// `conversation`. It admits no work and wakes nothing: without a
    /// connected runner there is nothing to release.
    pub async fn release_conversation(&self, conversation: &str) -> Result<(), HostError> {
        match self.link() {
            Ok(link) => link.release_conversation(conversation).await,
            Err(_) => Ok(()),
        }
    }

    async fn fs(
        &self,
        op: &'static str,
        message: impl FnOnce(String, Option<String>) -> Inbound,
    ) -> Result<FsResult, HostError> {
        let link = self.link()?;
        let _lease = self.admit()?;
        match link
            .call(Expected::Fs(op), |id| message(id, self.cwd()))
            .await?
        {
            Answer::Fs(result) => Ok(result),
            _ => Err(protocol("the runner answered another request")),
        }
    }

    async fn write_contents(
        &self,
        path: &str,
        contents: FileContents,
        options: WriteOptions,
    ) -> Result<(), HostError> {
        let input = self.write_pipe()?;
        let mut writer = input.writer().map_err(pipe_error)?;
        // What stopped the upload when its source failed: that failure, not
        // the runner's echo of it, is the one returned.
        let source_failure = RefCell::new(None);
        let upload = async {
            match contents {
                FileContents::Bytes(bytes) => {
                    if writer.write(bytes).await.is_ok() {
                        writer.end();
                    }
                }
                FileContents::Stream(mut stream) => {
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(bytes) => {
                                if writer.write(bytes).await.is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                writer.fail(&error.message);
                                *source_failure.borrow_mut() = Some(error);
                                return;
                            }
                        }
                    }
                    writer.end();
                }
            }
        };
        let ((), written) = tokio::join!(upload, self.write_from(path, &input, options));
        match written {
            Ok(()) => Ok(()),
            Err(error) => {
                input.fail(&error.message);
                Err(source_failure.into_inner().unwrap_or(error))
            }
        }
    }
}

/// A job to start.
pub struct JobStart {
    pub script: String,
    pub cwd: String,
    /// Exactly the variables the job's shell starts with, above the device's.
    pub env: BTreeMap<String, String>,
    pub context: CommandContext,
    pub caller: Option<JobCaller>,
    /// The commands the job may run; none for a job without declared
    /// commands.
    pub commands: Option<CommandSelection>,
    /// Pipes the job's standard input and output attach to, whose other
    /// ends are elsewhere.
    pub stdin: Option<PipeRef>,
    pub stdout: Option<PipeRef>,
}

/// One shell job on the runner.
#[derive(Clone)]
pub struct RemoteJob {
    id: String,
    shared: Rc<Shared<JobEnd>>,
    /// The connection it runs on; none when it ended before it started.
    link: Option<Link>,
}

impl RemoteJob {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The next chunk of the job's output view; none once the job ended and
    /// every chunk was taken.
    pub async fn next_output(&self) -> Option<ProcessOutput> {
        self.shared.next_output().await
    }

    /// The guidance of the latest declared command running in the job that
    /// declares one.
    pub fn running_hint(&self) -> Option<String> {
        self.shared.running_hint()
    }

    pub fn ended(&self) -> Option<JobEnd> {
        self.shared.ended()
    }

    pub async fn end(&self) -> JobEnd {
        self.shared.end().await
    }

    /// Writes to the job's standard input in frames of at most
    /// [`STDIN_CHUNK_BYTES`], in order; nothing once the job ended.
    pub async fn write_stdin(&self, bytes: Bytes) -> Result<(), HostError> {
        match live(&self.link, &self.shared) {
            Some(link) => {
                send_stdin(link, &bytes, |bytes| Inbound::JobStdin {
                    job_id: self.id.clone(),
                    bytes,
                })
                .await
            }
            None => Ok(()),
        }
    }

    pub async fn close_stdin(&self) -> Result<(), HostError> {
        match live(&self.link, &self.shared) {
            Some(link) => {
                link.send(&Inbound::JobStdinEnd {
                    job_id: self.id.clone(),
                })
                .await
            }
            None => Ok(()),
        }
    }

    pub async fn kill(&self, signal: wire::Signal) -> Result<(), HostError> {
        match live(&self.link, &self.shared) {
            Some(link) => {
                link.send(&Inbound::JobKill {
                    job_id: self.id.clone(),
                    signal: Some(signal),
                })
                .await
            }
            None => Ok(()),
        }
    }
}

/// The connection work runs on, while it runs.
fn live<'a, E: Clone>(link: &'a Option<Link>, shared: &Shared<E>) -> Option<&'a Link> {
    link.as_ref().filter(|_| shared.ended().is_none())
}

/// Sends `bytes` as ordered frames of at most [`STDIN_CHUNK_BYTES`], each in
/// the message `frame` makes of it.
async fn send_stdin(
    link: &Link,
    bytes: &[u8],
    frame: impl Fn(WireBytes) -> Inbound,
) -> Result<(), HostError> {
    for chunk in bytes.chunks(STDIN_CHUNK_BYTES) {
        link.send(&frame(WireBytes(chunk.to_vec()))).await?;
    }
    Ok(())
}

/// A page of the Host's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPage {
    pub lines: Vec<LogLine>,
    /// The cursor the next read continues from.
    pub next: u64,
}

/// A user stream to open, or a one-shot call.
pub struct ServiceRequest {
    pub context: CommandContext,
    pub package: PackageDescriptor,
    pub operation: String,
    /// The operation's arguments, as a command's invocation carries them.
    pub args: Option<Map<String, Value>>,
    /// Whether the invocation answers in JSON.
    pub json: Option<bool>,
    /// The conversation's directory, the invocation's working directory.
    pub cwd: String,
    /// Where the service's executable comes from, while the stream is open.
    pub resolver: Rc<dyn ArtifactResolver>,
}

/// How a user stream's invocation completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceEnd {
    pub exit_code: u8,
    /// The bounded tail of its standard error.
    pub stderr: String,
}

/// An open user stream. Closing it, or dropping it, ends the answers to its
/// artifact requests.
pub struct ServiceStream {
    link: Link,
    stream_id: String,
    ended: oneshot::Receiver<Result<ServiceEnd, HostError>>,
}

impl ServiceStream {
    /// How the invocation completed; an error when the runner went before it
    /// said.
    pub async fn done(&mut self) -> Result<ServiceEnd, HostError> {
        let link = &self.link;
        (&mut self.ended)
            .await
            .unwrap_or_else(|_| Err(link.offline()))
    }
}

impl Drop for ServiceStream {
    fn drop(&mut self) {
        let service = self
            .link
            .with_state(|state| state.remove_service(&self.stream_id));
        if let Some(service) = service {
            service.cancel.cancel();
        }
    }
}

/// Why a one-shot service call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServiceCallError {
    #[error(transparent)]
    Host(#[from] HostError),
    /// The invocation exited nonzero; `stderr` is the tail of what it wrote
    /// there, `stdout` its whole output.
    #[error("Service call exited with {exit_code}: {}", .stderr.trim())]
    Exited {
        exit_code: u8,
        stderr: String,
        stdout: Bytes,
    },
    #[error("Service answer exceeds {0} bytes")]
    TooLarge(usize),
}

impl Host for RemoteHost {
    fn key(&self) -> HostKey {
        self.0.key.clone()
    }

    fn default_cwd(&self) -> &str {
        &self.0.default_cwd
    }

    fn identity(&self) -> HostIdentity {
        match &*self.0.link.borrow() {
            DeviceLink::Online(link) => link.identity().clone(),
            DeviceLink::Offline {
                last: Some(identity),
            } => identity.clone(),
            DeviceLink::Offline { last: None } => HostIdentity {
                uid: 0,
                gid: 0,
                hostname: String::new(),
                home_dir: self.0.default_cwd.clone(),
            },
        }
    }

    fn fs(&self) -> &dyn HostFs {
        self
    }

    fn process(&self) -> &dyn HostProcess {
        self
    }
}

impl HostFs for RemoteHost {
    fn read_file<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<Bytes, HostError>> {
        Box::pin(async move { collect(self.read_pipe(path, ByteRange::default()).await?).await })
    }

    fn read_stream<'a>(
        &'a self,
        path: &'a str,
        range: ByteRange,
    ) -> LocalBoxFuture<'a, Result<ByteStream, HostError>> {
        Box::pin(async move {
            let reader = self.read_pipe(path, range).await?;
            Ok(reader
                .into_stream()
                .map(|chunk| chunk.map_err(|failure| HostError::interrupted(failure.to_string())))
                .boxed())
        })
    }

    fn write_file<'a>(
        &'a self,
        path: &'a str,
        contents: FileContents,
        options: WriteOptions,
    ) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(self.write_contents(path, contents, options))
    }

    fn exists<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<bool, HostError>> {
        Box::pin(async move {
            match self
                .fs("exists", |id, cwd| Inbound::FsExists {
                    id,
                    path: path.into(),
                    cwd,
                })
                .await?
            {
                FsResult::Exists(exists) => Ok(exists),
                _ => Err(mismatch()),
            }
        })
    }

    fn stat<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<FileStat, HostError>> {
        Box::pin(async move {
            match self
                .fs("stat", |id, cwd| Inbound::FsStat {
                    id,
                    path: path.into(),
                    cwd,
                })
                .await?
            {
                FsResult::Stat(stat) => file_stat(stat),
                _ => Err(mismatch()),
            }
        })
    }

    fn lstat<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<FileStat, HostError>> {
        Box::pin(async move {
            match self
                .fs("lstat", |id, cwd| Inbound::FsLstat {
                    id,
                    path: path.into(),
                    cwd,
                })
                .await?
            {
                FsResult::Lstat(stat) => file_stat(stat),
                _ => Err(mismatch()),
            }
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<Vec<DirEntry>, HostError>> {
        Box::pin(async move {
            let listing = self
                .fs("readdir", |id, cwd| Inbound::FsReaddir {
                    id,
                    path: path.into(),
                    cwd,
                    with_file_types: Some(true),
                })
                .await?;
            match listing {
                // The reply's shape is not tagged, so an empty listing reads
                // as the names a listing without file types answers.
                FsResult::Readdir(Readdir::Names(names)) if names.is_empty() => Ok(Vec::new()),
                FsResult::Readdir(Readdir::Entries(entries)) => Ok(entries
                    .into_iter()
                    .map(|entry| DirEntry {
                        kind: kind(
                            entry.is_file,
                            entry.is_directory,
                            entry.is_symbolic_link,
                            None,
                            None,
                        ),
                        name: entry.name,
                    })
                    .collect()),
                _ => Err(mismatch()),
            }
        })
    }

    fn mkdir<'a>(
        &'a self,
        path: &'a str,
        options: MkdirOptions,
    ) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            self.fs("mkdir", |id, cwd| Inbound::FsMkdir {
                id,
                path: path.into(),
                cwd,
                recursive: options.recursive.then_some(true),
            })
            .await
            .map(|_| ())
        })
    }

    fn rm<'a>(
        &'a self,
        path: &'a str,
        options: RmOptions,
    ) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            self.fs("rm", |id, cwd| Inbound::FsRm {
                id,
                path: path.into(),
                cwd,
                recursive: options.recursive.then_some(true),
                force: options.force.then_some(true),
            })
            .await
            .map(|_| ())
        })
    }

    fn cp<'a>(
        &'a self,
        path: &'a str,
        destination: &'a str,
        options: CpOptions,
    ) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            self.fs("cp", |id, cwd| Inbound::FsCp {
                id,
                path: path.into(),
                destination: destination.into(),
                cwd,
                recursive: options.recursive.then_some(true),
            })
            .await
            .map(|_| ())
        })
    }

    fn mv<'a>(
        &'a self,
        path: &'a str,
        destination: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            self.fs("mv", |id, cwd| Inbound::FsMv {
                id,
                path: path.into(),
                destination: destination.into(),
                cwd,
            })
            .await
            .map(|_| ())
        })
    }

    fn chmod<'a>(&'a self, path: &'a str, mode: u32) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            self.fs("chmod", |id, cwd| Inbound::FsChmod {
                id,
                path: path.into(),
                mode,
                cwd,
            })
            .await
            .map(|_| ())
        })
    }

    fn symlink<'a>(
        &'a self,
        target: &'a str,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            self.fs("symlink", |id, cwd| Inbound::FsSymlink {
                id,
                target: target.into(),
                path: path.into(),
                cwd,
            })
            .await
            .map(|_| ())
        })
    }

    fn link<'a>(
        &'a self,
        existing: &'a str,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            self.fs("link", |id, cwd| Inbound::FsLink {
                id,
                existing_path: existing.into(),
                path: path.into(),
                cwd,
            })
            .await
            .map(|_| ())
        })
    }

    fn readlink<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<String, HostError>> {
        Box::pin(async move {
            match self
                .fs("readlink", |id, cwd| Inbound::FsReadlink {
                    id,
                    path: path.into(),
                    cwd,
                })
                .await?
            {
                FsResult::Readlink(target) => Ok(target),
                _ => Err(mismatch()),
            }
        })
    }

    fn realpath<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<String, HostError>> {
        Box::pin(async move {
            match self
                .fs("realpath", |id, cwd| Inbound::FsRealpath {
                    id,
                    path: path.into(),
                    cwd,
                })
                .await?
            {
                FsResult::Realpath(real) => Ok(real),
                _ => Err(mismatch()),
            }
        })
    }

    fn utimes<'a>(
        &'a self,
        path: &'a str,
        accessed: Timestamp,
        modified: Timestamp,
    ) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            self.fs("utimes", |id, cwd| Inbound::FsUtimes {
                id,
                path: path.into(),
                atime: wire::Timestamp(accessed.as_millisecond()),
                mtime: wire::Timestamp(modified.as_millisecond()),
                cwd,
            })
            .await
            .map(|_| ())
        })
    }
}

impl HostProcess for RemoteHost {
    fn spawn(&self, request: SpawnRequest) -> LocalBoxFuture<'_, Result<Process, HostError>> {
        Box::pin(async move {
            // A retained process is retention, not activity: it holds no
            // admission and is not counted, so its machine may stop with it.
            let lease = if request.retained {
                None
            } else {
                self.admit()?
            };
            let shared = Shared::new();
            let Ok(link) = self.link() else {
                shared.finish(ProcessEnd::Lost("runner disconnected".into()));
                return Ok(process(shared, None, String::new()));
            };
            let id = uuid::Uuid::new_v4().simple().to_string();
            link.with_state(|state| {
                state.add_spawn(
                    id.clone(),
                    SpawnEntry {
                        shared: shared.clone(),
                        retained: request.retained,
                        _lease: lease,
                    },
                );
            });
            let (env, inherit_env) = match request.env {
                SpawnEnv::Inherit => (None, None),
                SpawnEnv::Exactly(env) => (
                    Some(
                        env.into_iter()
                            .map(|(name, value)| (name, Some(value)))
                            .collect(),
                    ),
                    None,
                ),
                SpawnEnv::Overlay(env) => (Some(env), Some(true)),
            };
            let spawn = Inbound::Spawn {
                spawn_id: id.clone(),
                command: request.command,
                args: (!request.args.is_empty()).then_some(request.args),
                cwd: request.cwd.or_else(|| self.cwd()),
                env,
                inherit_env,
                kill_process_group: None,
            };
            if let Err(error) = link.send(&spawn).await {
                link.with_state(|state| state.remove_spawn(&id));
                return Err(error);
            }
            Ok(process(shared, Some(link), id))
        })
    }
}

/// A process's handle over its shared output and end.
fn process(shared: Rc<Shared<ProcessEnd>>, link: Option<Link>, id: String) -> Process {
    let output = futures_util::stream::unfold(shared.clone(), |shared| async move {
        shared.next_output().await.map(|chunk| (chunk, shared))
    })
    .boxed_local();
    let exit = {
        let shared = shared.clone();
        Box::pin(async move { shared.end().await })
    };
    Process {
        output,
        control: Box::new(SpawnControl { link, id, shared }),
        exit,
    }
}

struct SpawnControl {
    link: Option<Link>,
    id: String,
    shared: Rc<Shared<ProcessEnd>>,
}

impl ProcessControl for SpawnControl {
    fn write_stdin(&self, bytes: Bytes) -> LocalBoxFuture<'_, Result<(), HostError>> {
        Box::pin(async move {
            match live(&self.link, &self.shared) {
                Some(link) => {
                    send_stdin(link, &bytes, |bytes| Inbound::SpawnStdin {
                        spawn_id: self.id.clone(),
                        bytes,
                    })
                    .await
                }
                None => Ok(()),
            }
        })
    }

    fn close_stdin(&self) -> LocalBoxFuture<'_, Result<(), HostError>> {
        Box::pin(async move {
            match live(&self.link, &self.shared) {
                Some(link) => {
                    link.send(&Inbound::SpawnStdinEnd {
                        spawn_id: self.id.clone(),
                    })
                    .await
                }
                None => Ok(()),
            }
        })
    }

    fn kill(&self, signal: Signal) -> LocalBoxFuture<'_, Result<(), HostError>> {
        Box::pin(async move {
            let signal = match signal {
                Signal::Terminate => wire::Signal::Terminate,
                Signal::Kill => wire::Signal::Kill,
            };
            match live(&self.link, &self.shared) {
                Some(link) => {
                    link.send(&Inbound::SpawnKill {
                        spawn_id: self.id.clone(),
                        signal: Some(signal),
                    })
                    .await
                }
                None => Ok(()),
            }
        })
    }
}

/// Sends the request `message` builds around an id and a pipe the runner
/// fills, and returns the pipe's reader once the runner accepted. A refusal
/// fails the pipe: its bytes will never come.
async fn filled(
    link: &Link,
    expected: Expected,
    message: impl FnOnce(String, PipeRef) -> Inbound,
) -> Result<PipeReader, HostError> {
    let pipe = link.pipes().from_device(link.device());
    let reader = pipe.reader().map_err(pipe_error)?;
    let output = pipe.wire_ref();
    match link.call(expected, |id| message(id, output)).await {
        Ok(_) => Ok(reader),
        Err(error) => {
            pipe.fail(&error.message);
            Err(error)
        }
    }
}

/// Reads a pipe to its end.
async fn collect(mut reader: PipeReader) -> Result<Bytes, HostError> {
    let mut bytes = BytesMut::new();
    while let Some(chunk) = reader.next().await {
        bytes.extend_from_slice(
            &chunk.map_err(|failure| HostError::interrupted(failure.to_string()))?,
        );
    }
    Ok(bytes.freeze())
}

fn file_stat(stat: wire::FileStat) -> Result<FileStat, HostError> {
    Ok(FileStat {
        kind: kind(
            stat.is_file,
            stat.is_directory,
            stat.is_symbolic_link,
            stat.is_character_device,
            stat.is_fifo,
        ),
        mode: stat.mode,
        size: stat.size,
        modified: Timestamp::from_millisecond(stat.mtime.0)
            .map_err(|error| protocol(&error.to_string()))?,
    })
}

fn kind(
    file: bool,
    directory: bool,
    symlink: bool,
    character_device: Option<bool>,
    fifo: Option<bool>,
) -> FileKind {
    if symlink {
        FileKind::Symlink
    } else if directory {
        FileKind::Directory
    } else if file {
        FileKind::File
    } else if character_device == Some(true) {
        FileKind::CharacterDevice
    } else if fifo == Some(true) {
        FileKind::Fifo
    } else {
        FileKind::Other
    }
}

fn pipe_error(error: PipeError) -> HostError {
    HostError::interrupted(error.to_string())
}

fn protocol(message: &str) -> HostError {
    HostError::new(HostErrorKind::Protocol, message)
}

fn mismatch() -> HostError {
    protocol("the runner answered another operation")
}

/// The account a runner reported in its hello.
pub fn host_identity(identity: &wire::HostIdentity) -> HostIdentity {
    HostIdentity {
        uid: identity.uid,
        gid: identity.gid,
        hostname: identity.hostname.clone(),
        home_dir: identity.home_dir.clone(),
    }
}
