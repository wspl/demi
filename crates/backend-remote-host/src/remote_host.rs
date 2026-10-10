//! `RemoteHost`: a Host over its device's runner connection
//! (`runner.md` § Host operations). Every filesystem operation is one request
//! and its reply, with a file's contents in a pipe beside it; processes and
//! jobs stream over their own messages. A Host is a value: two handles with
//! equal keys are the same Host, whichever connection serves them, and a
//! handle made while its runner is away serves once it is back.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use bytes::{Bytes, BytesMut};
use demi_command_protocol::{
    ArtifactLocation, CommandContext, MAX_MEDIUM_BYTES, PackageArtifact, PackageDescriptor,
};
use demi_host_interface::{
    ByteRange, ByteStream, CpOptions, DirEntry, FileContents, FileKind, FileStat, Host, HostError,
    HostErrorKind, HostFs, HostIdentity, HostKey, HostProcess, JobCaller, MkdirOptions, Process,
    ProcessControl, ProcessEnd, ProcessOutput, RmOptions, Signal, SpawnEnv, SpawnRequest,
    WhenExists, WholeOutput, WriteOptions,
};
use demi_runner_protocol::wire::{
    self, FsResult, GitChanges, GitResult, Inbound, LogLine, PipeRef, STDIN_CHUNK_BYTES, WireBytes,
};
use demi_shared_gates::GateLease;
use demi_shared_types::Timestamp;
use futures_util::{StreamExt, future::LocalBoxFuture};
use serde_json::{Map, Value};
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    ArtifactResolver, HostWatch, Link,
    device_jobs::{Claimed, DeviceJobs, NewJob},
    link::{
        Answer, Expected, JobEnd, JobMedium, JobOrigin, JobOutput, ServiceEntry, Shared,
        SpawnEntry,
    },
    manifest::CommandSelection,
    output_records::decode_output,
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

/// A file a read opened: its metadata and version then, and its bytes.
pub struct OpenedRead {
    pub stat: FileStat,
    pub version: String,
    pub body: PipeReader,
}

/// A pipe's bytes as a Host's byte stream: a pipe that fails interrupts it.
pub fn byte_stream(reader: PipeReader) -> ByteStream {
    reader
        .into_stream()
        .map(|chunk| chunk.map_err(|failure| HostError::interrupted(failure.to_string())))
        .boxed()
}

/// What a read that holds a version answers.
pub enum ConditionalRead {
    /// The file still has the version held: no bytes move.
    Unchanged { stat: FileStat, version: String },
    Opened(OpenedRead),
}

/// A Host over a runner connection.
#[derive(Clone)]
pub struct RemoteHost(Rc<Inner>);

struct Inner {
    key: HostKey,
    default_cwd: String,
    link: watch::Receiver<DeviceLink>,
    /// The device's jobs, which outlive each connection.
    jobs: DeviceJobs,
    admission: Admission,
}

impl RemoteHost {
    pub fn new(
        key: HostKey,
        default_cwd: String,
        link: watch::Receiver<DeviceLink>,
        jobs: DeviceJobs,
        admission: Admission,
    ) -> Self {
        Self(Rc::new(Inner {
            key,
            default_cwd,
            link,
            jobs,
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

    /// A new job's id, which a job's records name before it starts.
    pub fn job_id() -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }

    /// Runs `script` as one shell job. Offline, the job has ended already;
    /// a Host that admits no work refuses it.
    pub async fn start_job(&self, job: JobStart) -> Result<RemoteJob, HostError> {
        let lease = self.admit()?;
        let id = job.id.clone();
        let origin = Rc::new(JobOrigin {
            host: self.0.key.clone(),
            context: job.context.clone(),
            caller: job.caller.clone(),
        });
        let entry = NewJob {
            shared: Shared::new(),
            media: Rc::default(),
            origin: Some(origin),
            commands: job.commands.clone(),
            lease,
        };
        let Ok(link) = self.link() else {
            entry.shared.finish(JobEnd::lost("runner disconnected"));
            let claimed = Claimed {
                shared: entry.shared,
                media: entry.media,
                attachments: watch::channel(0).1,
            };
            return Ok(self.job(id, claimed));
        };
        let claimed = self.0.jobs.start(id.clone(), entry, &link);
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
            self.0.jobs.forget(&id);
            claimed.shared.finish(JobEnd::lost(&error.message));
            return Err(error);
        }
        Ok(self.job(id, claimed))
    }

    /// Takes up the job `id`, which the backend recorded running on this
    /// Host before it restarted (`sessions-and-targets.md` § Recovery and
    /// persistence): its runner's hello says whether it goes on, ended or
    /// was lost, now or when the runner connects.
    /// Its rpc calls act for `caller` with `context`; without a context
    /// they are refused.
    pub fn adopt_job(
        &self,
        id: String,
        context: Option<CommandContext>,
        caller: Option<JobCaller>,
        commands: Option<CommandSelection>,
    ) -> RemoteJob {
        let origin = context.map(|context| {
            Rc::new(JobOrigin {
                host: self.0.key.clone(),
                context,
                caller,
            })
        });
        // A Host that admits no work holds nothing for a job that runs
        // already.
        let lease = self.admit().ok().flatten();
        let claimed = self.0.jobs.claim(
            &id,
            NewJob {
                shared: Shared::new(),
                media: Rc::default(),
                origin,
                commands,
                lease,
            },
        );
        self.job(id, claimed)
    }

    fn job(&self, id: String, claimed: Claimed) -> RemoteJob {
        RemoteJob {
            id,
            shared: claimed.shared,
            media: claimed.media,
            attachments: claimed.attachments,
            device: self.0.link.clone(),
        }
    }

    /// A pipe the device's runner fills with `range` of the file, once the
    /// file is open, with the file's metadata and version then, so no `stat`
    /// precedes a read (`runner.md` § Host operations): a file that cannot
    /// be opened, or is not a regular file, fails here, before any byte. A
    /// range is read up to the end the file had when it was opened. Dropping
    /// the reader stops the read.
    pub async fn read_pipe(&self, path: &str, range: ByteRange) -> Result<OpenedRead, HostError> {
        opened(self.read_pipe_held(self.cwd(), path, range, None).await?)
    }

    /// [`RemoteHost::read_pipe`], with a relative `path` resolved against
    /// `cwd` instead of the directory the Host's work starts in, such as the
    /// directory a command was invoked in.
    pub async fn read_pipe_in(
        &self,
        cwd: &str,
        path: &str,
        range: ByteRange,
    ) -> Result<OpenedRead, HostError> {
        opened(
            self.read_pipe_held(Some(cwd.to_owned()), path, range, None)
                .await?,
        )
    }

    /// The size and SHA-256 of the file at `path` beside `cwd`, none of
    /// whose bytes travel, or its size alone when it is over `limit` bytes
    /// (`runner.md` § File contents).
    pub async fn hash_file_in(
        &self,
        cwd: &str,
        path: &str,
        limit: u64,
    ) -> Result<wire::FileHash, HostError> {
        let link = self.link()?;
        let _lease = self.admit()?;
        let hashed = link
            .call(Expected::Fs("hashFile"), |id| Inbound::FsHashFile {
                id,
                path: path.into(),
                cwd: Some(cwd.to_owned()),
                limit,
            })
            .await?;
        match hashed {
            Answer::Fs(FsResult::HashFile(hash)) => Ok(hash),
            _ => Err(mismatch()),
        }
    }

    /// [`RemoteHost::read_pipe`], except that a file that still has the
    /// version `held` streams nothing (`runner.md` § File contents).
    pub async fn read_pipe_unless(
        &self,
        path: &str,
        range: ByteRange,
        held: &str,
    ) -> Result<ConditionalRead, HostError> {
        self.read_pipe_held(self.cwd(), path, range, Some(held)).await
    }

    async fn read_pipe_held(
        &self,
        cwd: Option<String>,
        path: &str,
        range: ByteRange,
        held: Option<&str>,
    ) -> Result<ConditionalRead, HostError> {
        let link = self.link()?;
        let _lease = self.admit()?;
        let (answer, reader) = answered(&link, Expected::Fs("readFile"), |id, output| {
            Inbound::FsReadFile {
                id,
                path: path.into(),
                cwd,
                offset: (range.offset > 0).then_some(range.offset),
                length: range.length,
                version: held.map(str::to_owned),
                output,
            }
        })
        .await?;
        let Answer::Fs(FsResult::ReadFile(opened)) = answer else {
            return Err(mismatch());
        };
        let stat = file_stat(opened.stat)?;
        if opened.unchanged {
            // The runner reports the unused pipe's end.
            return Ok(ConditionalRead::Unchanged {
                stat,
                version: opened.version,
            });
        }
        Ok(ConditionalRead::Opened(OpenedRead {
            stat,
            version: opened.version,
            body: reader,
        }))
    }

    /// Follows the Host's watch of `path`, and of what lies below it when
    /// `recursive` (`runner.md` § Watching files): pages that watch the
    /// same path share one. It holds no admission, so it keeps no Cloud
    /// awake.
    pub fn watch(&self, path: &str, recursive: bool) -> Result<HostWatch, HostError> {
        Ok(self.link()?.watch_files(path, recursive))
    }

    /// A pipe to the device's runner that this process fills.
    pub fn write_pipe(&self) -> Result<Pipe, HostError> {
        let link = self.link()?;
        Ok(link.pipes().to_device(link.device()))
    }

    /// Writes the file from `input`, a pipe from [`RemoteHost::write_pipe`]:
    /// the runner puts the file in place once it read the pipe to its end,
    /// and then replies with the name it has. A pipe that fails leaves the
    /// file as it was; a path taken that the write may not have refuses it
    /// before the pipe is read.
    pub async fn write_from(
        &self,
        path: &str,
        input: &Pipe,
        options: WriteOptions,
    ) -> Result<String, HostError> {
        let link = self.link()?;
        let _lease = self.admit()?;
        let written = link
            .call(Expected::Fs("writeFile"), |id| Inbound::FsWriteFile {
                id,
                path: path.into(),
                cwd: self.cwd(),
                exists: match options.exists {
                    WhenExists::Replace => wire::WriteExists::Replace,
                    WhenExists::Refuse => wire::WriteExists::Refuse,
                    WhenExists::Rename => wire::WriteExists::Rename,
                },
                input: input.wire_ref(),
            })
            .await?;
        match written {
            Answer::Fs(FsResult::WriteFile(name)) => Ok(name),
            _ => Err(mismatch()),
        }
    }

    /// Writes the directory `path` whole with one request (`runner.md`
    /// § Host operations): each of `files` by its path inside it, with its
    /// mode and bytes, and every directory in it with `directory_mode`. The
    /// runner puts it in place once every file is written; a directory at
    /// `path` already is kept.
    pub async fn write_directory(
        &self,
        path: &str,
        files: Vec<DirectoryFile>,
        directory_mode: u32,
    ) -> Result<(), HostError> {
        let link = self.link()?;
        let _lease = self.admit()?;
        let input = link.pipes().to_device(link.device());
        let mut writer = input.writer().map_err(pipe_error)?;
        let (listing, contents): (Vec<wire::DirectoryFile>, Vec<Bytes>) = files
            .into_iter()
            .map(|file| {
                let listed = wire::DirectoryFile {
                    path: file.path,
                    mode: file.mode,
                };
                (listed, file.bytes)
            })
            .unzip();
        let upload = async {
            for bytes in contents {
                let length = Bytes::copy_from_slice(&(bytes.len() as u64).to_be_bytes());
                if writer.write(length).await.is_err() || writer.write(bytes).await.is_err() {
                    return;
                }
            }
            writer.end();
        };
        let written = link.call(Expected::Fs("writeDirectory"), |id| Inbound::FsWriteDirectory {
            id,
            path: path.into(),
            files: listing,
            directory_mode,
            input: input.wire_ref(),
        });
        let ((), written) = tokio::join!(upload, written);
        if let Err(error) = &written {
            input.fail(&error.message);
        }
        written.map(|_| ())
    }

    /// Removes each of `paths` with everything in it, with one request; a
    /// read-only directory is made writable first, and a path that is not
    /// there is removed already.
    pub async fn remove_all(&self, paths: &[String]) -> Result<(), HostError> {
        self.fs("removeAll", |id, _| Inbound::FsRemoveAll {
            id,
            paths: paths.to_vec(),
        })
        .await
        .map(|_| ())
    }

    /// Reads several files with one request (`runner.md` § Host
    /// operations): each file's bytes, or why it was not read, in the
    /// order of `paths`. A file over `limit` bytes is not read.
    pub async fn read_files(
        &self,
        paths: &[String],
        limit: u64,
    ) -> Result<Vec<Result<Bytes, HostError>>, HostError> {
        let link = self.link()?;
        let _lease = self.admit()?;
        let (answer, reader) = answered(&link, Expected::Fs("readFiles"), |id, output| {
            Inbound::FsReadFiles {
                id,
                paths: paths.to_vec(),
                cwd: self.cwd(),
                limit,
                output,
            }
        })
        .await?;
        let Answer::Fs(FsResult::ReadFiles(reads)) = answer else {
            return Err(mismatch());
        };
        split_reads(reader, reads, paths.len(), limit).await
    }

    /// Looks at several paths with one request (`runner.md` § Host
    /// operations): each answers what is there, in the order of `paths`.
    /// A path the Host could not read answers so; a failure that is not
    /// about one path fails the look.
    pub async fn look(&self, paths: &[LookAt]) -> Result<Vec<Look>, HostError> {
        let link = self.link()?;
        let _lease = self.admit()?;
        let wire_paths: Vec<wire::LookPath> = paths
            .iter()
            .map(|wanted| wire::LookPath {
                path: wanted.path.clone(),
                limit: wanted.limit,
            })
            .collect();
        let bound: u64 = paths.iter().map(|wanted| wanted.limit).sum();
        // A pipe only when some file's bytes are wanted.
        let (answer, bytes) = if bound > 0 {
            let (answer, reader) = answered(&link, Expected::Fs("look"), |id, output| {
                Inbound::FsLook {
                    id,
                    paths: wire_paths,
                    cwd: self.cwd(),
                    output: Some(output),
                }
            })
            .await?;
            let limit = usize::try_from(bound).unwrap_or(usize::MAX);
            (answer, collect_pipe(reader, limit).await?)
        } else {
            let answer = link
                .call(Expected::Fs("look"), |id| Inbound::FsLook {
                    id,
                    paths: wire_paths,
                    cwd: self.cwd(),
                    output: None,
                })
                .await?;
            (answer, Bytes::new())
        };
        let Answer::Fs(FsResult::Look(looked)) = answer else {
            return Err(mismatch());
        };
        if looked.len() != paths.len() {
            return Err(protocol("the runner answered another number of paths"));
        }
        let mut offset: usize = 0;
        looked
            .into_iter()
            .map(|looked| {
                Ok(match looked {
                    wire::Looked::Missing => Look::Missing,
                    wire::Looked::Directory { entries } => Look::Directory(
                        entries
                            .into_iter()
                            .map(dir_entry)
                            .collect::<Result<_, _>>()?,
                    ),
                    wire::Looked::File { size, read } => {
                        let end = usize::try_from(read)
                            .ok()
                            .and_then(|read| offset.checked_add(read))
                            .filter(|end| *end <= bytes.len())
                            .ok_or_else(|| protocol("the look's bytes are shorter than its answer"))?;
                        let file = bytes.slice(offset..end);
                        offset = end;
                        Look::File { bytes: file, size }
                    }
                    wire::Looked::Other => Look::Other,
                    wire::Looked::Unreadable { code, message } => {
                        Look::Unreadable(HostError::failed(code, message))
                    }
                })
            })
            .collect()
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
        collect_pipe(reader, usize::MAX).await
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
                    attached: request.attached.clone(),
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
            return Err(broken(&link, failure).into());
        }
        writer.end();
        let mut answer = BytesMut::new();
        while let Some(chunk) = reader.next().await {
            let chunk = chunk.map_err(|failure| broken(&link, failure))?;
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
    ) -> Result<String, HostError> {
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
            Ok(name) => Ok(name),
            Err(error) => {
                input.fail(&error.message);
                Err(source_failure.into_inner().unwrap_or(error))
            }
        }
    }
}

/// One file of a directory a write puts in place whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryFile {
    /// Inside the directory, with `/` between its parts.
    pub path: String,
    pub mode: u32,
    pub bytes: Bytes,
}

/// One path a look names, and the most bytes of a file there to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookAt {
    pub path: String,
    pub limit: u64,
}

/// What a look found at one path. A symbolic link at the path is followed;
/// a directory's entries name a link as a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Look {
    Missing,
    Directory(Vec<DirEntry>),
    /// A file's first bytes, at most the look's limit, and its size.
    File { bytes: Bytes, size: u64 },
    /// Neither a file nor a directory.
    Other,
    /// The Host could not read it.
    Unreadable(HostError),
}

/// A job to start.
pub struct JobStart {
    /// Its id, from [`RemoteHost::job_id`].
    pub id: String,
    pub script: String,
    /// Where the job starts; one that does not exist fails it before its
    /// script runs (`runner.md` § Shell jobs).
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

/// One shell job on the runner, over whichever connection of its device
/// serves it (`runner.md` § Command lifetime).
#[derive(Clone)]
pub struct RemoteJob {
    id: String,
    shared: Rc<Shared<JobEnd, JobOutput>>,
    /// The media its commands returned, as the runner announced them.
    media: Rc<RefCell<Vec<JobMedium>>>,
    /// Changes each time the job goes on over a new connection.
    attachments: watch::Receiver<u64>,
    /// The device's connection.
    device: watch::Receiver<DeviceLink>,
}

impl RemoteJob {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The connection that serves the job's device now.
    fn link(&self) -> Option<Link> {
        match &*self.device.borrow() {
            DeviceLink::Online(link) if !link.is_closed() => Some(link.clone()),
            _ => None,
        }
    }

    /// The connection that serves the job while it runs.
    fn live(&self) -> Option<Link> {
        self.link().filter(|_| self.shared.ended().is_none())
    }

    /// The next message of the job's output views; none once the job ended
    /// and every message was taken.
    pub async fn next_output(&self) -> Option<JobOutput> {
        self.shared.next_output().await
    }

    /// Resolves once the job goes on over a new connection of its device,
    /// which knows nothing of how the backend followed it.
    pub async fn reattached(&mut self) {
        if self.attachments.changed().await.is_err() {
            // A job the device forgot goes on over no other connection.
            std::future::pending::<()>().await;
        }
    }

    /// Starts or stops the runner's sending of the job's output beyond each
    /// stream's first `JOB_VIEW_BYTES` (`runner.md` § Pipes and output);
    /// nothing once the job ended or while no connection serves it.
    pub async fn follow(&self, follow: bool) -> Result<(), HostError> {
        match self.live() {
            Some(link) => {
                link.send(&Inbound::JobFollow {
                    job_id: self.id.clone(),
                    follow,
                })
                .await
            }
            None => Ok(()),
        }
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
    /// [`STDIN_CHUNK_BYTES`], in order; nothing once the job ended, and an
    /// error while no connection serves it.
    pub async fn write_stdin(&self, bytes: Bytes) -> Result<(), HostError> {
        if self.shared.ended().is_some() {
            return Ok(());
        }
        let link = self.connected()?;
        send_stdin(&link, &bytes, |bytes| Inbound::JobStdin {
            job_id: self.id.clone(),
            bytes,
        })
        .await
    }

    pub async fn close_stdin(&self) -> Result<(), HostError> {
        match self.live() {
            Some(link) => {
                link.send(&Inbound::JobStdinEnd {
                    job_id: self.id.clone(),
                })
                .await
            }
            None => Ok(()),
        }
    }

    /// The connection that serves the job's device, or why there is none.
    fn connected(&self) -> Result<Link, HostError> {
        self.link()
            .ok_or_else(|| HostError::offline("the job's runner is not connected"))
    }

    /// The job's kept output as it stands (`runner.md` § Pipes and output):
    /// while the job runs, and after it ended until its release.
    pub async fn read_output(&self) -> Result<WholeOutput, HostError> {
        let link = self.connected()?;
        let reader = filled_job_read(&link, &self.id).await?;
        let bytes = collect_pipe(reader, wire::JOB_KEPT_READ_BYTES).await?;
        decode_output(&bytes, None)
            .map_err(|error| protocol(&format!("the job's kept output does not decode: {error}")))
    }

    /// The media the job's commands returned so far, in order: every one
    /// once the job ended, even when its connection was lost.
    pub fn media(&self) -> Vec<JobMedium> {
        self.media.borrow().clone()
    }

    /// The bytes of the media `numbers` the job keeps, with one request
    /// (`runner.md` § Pipes and output): while the job runs, and after it
    /// ended until its release. Each answers its bytes or why it was not
    /// read, in the order of `numbers`.
    pub async fn read_media(
        &self,
        numbers: &[u32],
    ) -> Result<Vec<Result<Bytes, HostError>>, HostError> {
        let link = self.connected()?;
        let (answer, reader) = answered(&link, Expected::JobMediaRead, |id, output| {
            Inbound::JobMediaRead {
                id,
                job_id: self.id.clone(),
                numbers: numbers.to_vec(),
                output,
            }
        })
        .await?;
        let Answer::Read(reads) = answer else {
            return Err(protocol("the runner answered another request"));
        };
        split_reads(reader, reads, numbers.len(), MAX_MEDIUM_BYTES).await
    }

    /// Tells the runner that the backend has what it needs of the ended job,
    /// so its directory goes. Without a connection, the runner's next hello
    /// lists the job again, and its exit releases it then.
    pub async fn release(&self) {
        let Some(link) = self.link() else {
            return;
        };
        if let Err(error) = link
            .send(&Inbound::JobRelease {
                job_id: self.id.clone(),
            })
            .await
        {
            tracing::debug!(job = %self.id, "job release not sent: {error}");
        }
    }

    pub async fn kill(&self, signal: wire::Signal) -> Result<(), HostError> {
        match self.live() {
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

/// The connection a raw process runs on, while it runs.
fn live<'a, E: Clone, C>(link: &'a Option<Link>, shared: &Shared<E, C>) -> Option<&'a Link> {
    link.as_ref().filter(|_| shared.ended().is_none())
}

/// Asks the runner to fill a pipe with `job`'s kept output, and returns the
/// pipe's reader once the runner accepted.
pub(crate) async fn filled_job_read(link: &Link, job: &str) -> Result<PipeReader, HostError> {
    filled(link, Expected::JobRead, |id, output| Inbound::JobRead {
        id,
        job_id: job.to_owned(),
        output,
    })
    .await
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
    /// Where the package's artifacts come from, while the stream is open.
    pub resolver: Rc<dyn ArtifactResolver>,
    /// Other artifacts the invocation may install, each with its location,
    /// such as the downloads of a release the backend chose
    /// (`native-runtime.md` § Where an artifact comes from).
    pub attached: Vec<AttachedArtifact>,
}

/// An artifact a user stream may install beside its package's own, and
/// where it is downloaded from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedArtifact {
    pub artifact: PackageArtifact,
    pub location: ArtifactLocation,
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
        Box::pin(async move {
            collect_pipe(
self.read_pipe(path, ByteRange::default()).await?.body,
                usize::MAX,
            )
            .await
        })
    }

    fn read_stream<'a>(
        &'a self,
        path: &'a str,
        range: ByteRange,
    ) -> LocalBoxFuture<'a, Result<ByteStream, HostError>> {
        Box::pin(async move {
            Ok(byte_stream(self.read_pipe(path, range).await?.body))
        })
    }

    fn write_file<'a>(
        &'a self,
        path: &'a str,
        contents: FileContents,
        options: WriteOptions,
    ) -> LocalBoxFuture<'a, Result<String, HostError>> {
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
                })
                .await?;
            match listing {
                FsResult::Readdir(entries) => entries.into_iter().map(dir_entry).collect(),
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
                descriptors: (!request.descriptors.is_empty()).then(|| {
                    request
                        .descriptors
                        .into_iter()
                        .map(|descriptor| wire::Descriptor {
                            fd: descriptor.fd,
                            bytes: wire::WireBytes(descriptor.bytes.to_vec()),
                        })
                        .collect()
                }),
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
fn process(
    shared: Rc<Shared<ProcessEnd, ProcessOutput>>,
    link: Option<Link>,
    id: String,
) -> Process {
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
    shared: Rc<Shared<ProcessEnd, ProcessOutput>>,
}

impl Drop for SpawnControl {
    /// Nobody controls a process whose handle is gone, so one that has not
    /// ended is killed; the runner is asked without waiting (`runner.md`
    /// § Host operations).
    fn drop(&mut self) {
        if let Some(link) = live(&self.link, &self.shared) {
            link.post(&Inbound::SpawnKill {
                spawn_id: self.id.clone(),
                signal: Some(wire::Signal::Kill),
            });
        }
    }
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
    answered(link, expected, message)
        .await
        .map(|(_, reader)| reader)
}

/// [`filled`], with the runner's answer.
async fn answered(
    link: &Link,
    expected: Expected,
    message: impl FnOnce(String, PipeRef) -> Inbound,
) -> Result<(Answer, PipeReader), HostError> {
    let pipe = link.pipes().from_device(link.device());
    let reader = pipe.reader().map_err(pipe_error)?;
    let output = pipe.wire_ref();
    match link.call(expected, |id| message(id, output)).await {
        Ok(answer) => Ok((answer, reader)),
        Err(error) => {
            pipe.fail(&error.message);
            Err(error)
        }
    }
}

/// Each file of a read of several: its bytes, which the pipe carries after
/// their length (`wire::FileRead`), or why the runner did not read it. The
/// answer names `count` files, each read one of at most `limit` bytes.
async fn split_reads(
    reader: PipeReader,
    reads: Vec<wire::FileRead>,
    count: usize,
    limit: u64,
) -> Result<Vec<Result<Bytes, HostError>>, HostError> {
    if reads.len() != count {
        return Err(protocol("the runner answered another number of files"));
    }
    let read = reads
        .iter()
        .filter(|read| matches!(read, wire::FileRead::Read))
        .count();
    let length = std::mem::size_of::<u64>();
    let bound = usize::try_from(limit)
        .unwrap_or(usize::MAX)
        .saturating_add(length)
        .saturating_mul(read);
    let mut bytes = collect_pipe(reader, bound).await?;
    let files = reads
        .into_iter()
        .map(|read| match read {
            wire::FileRead::Read => {
                if bytes.len() < length {
                    return Err(protocol("a file of the read is missing its length"));
                }
                let header = bytes.split_to(length);
                let size = u64::from_be_bytes(header[..].try_into().expect("eight bytes"));
                let size = usize::try_from(size)
                    .ok()
                    .filter(|size| *size <= bytes.len())
                    .ok_or_else(|| protocol("a file of the read is shorter than its length"))?;
                Ok(Ok(bytes.split_to(size)))
            }
            wire::FileRead::Failed { code, message } => Ok(Err(HostError::failed(code, message))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !bytes.is_empty() {
        return Err(protocol("the read sent more than its files"));
    }
    Ok(files)
}

/// Reads a pipe to its end, which comes within `limit` bytes.
/// A read that named no version it holds, which the runner always opens.
fn opened(read: ConditionalRead) -> Result<OpenedRead, HostError> {
    match read {
        ConditionalRead::Opened(opened) => Ok(opened),
        ConditionalRead::Unchanged { .. } => Err(mismatch()),
    }
}

/// The bytes of `reader` to its end; more than `limit` of them is the
/// runner's protocol error.
pub async fn collect_pipe(mut reader: PipeReader, limit: usize) -> Result<Bytes, HostError> {
    let mut bytes = BytesMut::new();
    while let Some(chunk) = reader.next().await {
        let chunk = chunk.map_err(|failure| HostError::interrupted(failure.to_string()))?;
        if bytes.len() + chunk.len() > limit {
            return Err(protocol(&format!(
                "the runner sent more than {limit} bytes"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes.freeze())
}

fn dir_entry(entry: wire::DirEntry) -> Result<DirEntry, HostError> {
    Ok(DirEntry {
        kind: kind(
            entry.is_file,
            entry.is_directory,
            entry.is_symbolic_link,
            None,
            None,
        ),
        size: entry.size,
        modified: timestamp(entry.mtime)?,
        name: entry.name,
    })
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
        modified: timestamp(stat.mtime)?,
    })
}

/// A time on the wire as a timestamp; one out of its range is the runner's
/// error.
fn timestamp(time: wire::Timestamp) -> Result<Timestamp, HostError> {
    Timestamp::from_millisecond(time.0).map_err(|error| protocol(&error.to_string()))
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

/// Why a call's bytes stopped: the device going offline when its runner
/// went away, which fails every pipe it held, so the caller says what
/// happened rather than that a pipe broke; else the pipe itself broke.
fn broken(link: &Link, failure: impl ToString) -> HostError {
    if link.is_closed() {
        return link.offline();
    }
    HostError::interrupted(failure.to_string())
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
