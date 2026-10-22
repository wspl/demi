//! `RemoteShellEnvironment`: the shell behind the `shell_*` tools on a Host
//! reached through its runner (`crates/runner-shell/src/lib.rs`).
//!
//! Every exec is one job. The record holds two views of its output:
//!
//! - The model's view: the head while the job runs and the whole output once it
//!   ended.
//! - The pages' view, which also holds the newest output the runner sends while
//!   a page watches and the job is therefore followed
//!   (`crates/agent-server/src/server/tree/live.rs`).
//!
//! Every job starts in its node's working directory on the Host, and nothing of
//! an earlier one carries over.
//!
//! A command outlives the environment: its keeper records it running before its
//! job starts, a disposed environment lets its jobs run on, and the next
//! environment of its node takes them up again
//! (`crates/backend-remote-host/src/device_jobs.rs`).
//!
//! When a command ends, the keeper stores its whole output before any result
//! reports the end, so the result and the pages show the same lines.
//!
//! - The backend stores what it received when that is the whole output: each
//!   stream within its first 8 KiB and every byte received. Otherwise it reads
//!   the Host's kept output once.
//! - A command lost with its connection keeps what the backend received up to
//!   the first byte it missed, followed by a line that counts the bytes lost.
//!
//! The stored output is never part of a request, so storing or removing it
//! changes no request.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use demi_command_protocol::{CommandContext, EditKind as JobEditKind};
use demi_host_interface::{
    BinaryOutput, CommandMedium, CommandRecord, CommandSet, CommandStatus, DEFAULT_BINARY_LIMIT_BYTES,
    DEFAULT_OUTPUT_LIMIT_BYTES, EditedFiles, Ending, ExecRequest, Host, HostError, HostErrorKind, HostKey,
    JobCaller, Missing, Numbers, OutputRecord, PageFeed, PageView, ProcessEnd, Seen,
    ShellEnvironment, ShellError, SpawnErrorKind, Streams, TakenUp, Unreachable, UnsentStart, WholeOutput,
    binary_line,
};
use demi_runner_protocol::{
    manifest::ManifestError,
    wire::{self, JobFileChange},
};
use demi_shared_types::{
    BlobRef, CommandEnd, CommandId, EditCopies, EditKind, EditSegment, EditedFile, PathChange,
    Sequence, StreamKind,
};
use futures_util::future::LocalBoxFuture;
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    CommandCatalog, JobEnd, JobMedium, JobOutput, JobStart, RemoteHost, RemoteJob,
    device_jobs::Newest, manifest::CommandSelection,
};

/// The product's host access around one job: it holds what the Host needs,
/// such as the conversation's file gate and a Cloud's admission, through the
/// job's start, output and edit publication, and refuses when the
/// conversation's Host is no longer `host`. `cancel` ends its waits before
/// the job starts. While the job's start waits for its Host, a paired
/// device whose runner is away, it says so through `waiting`.
pub trait HostAccess {
    fn run_job<'a>(
        &'a self,
        host: &'a HostKey,
        cancel: CancellationToken,
        waiting: JobWaiting<'a>,
        job: LocalBoxFuture<'a, ()>,
    ) -> LocalBoxFuture<'a, Result<(), HostError>>;
}

/// How a job whose start may wait for its Host (`crates/backend-host-access/src/access.rs`) shows it: waiting for the device it names, and no
/// longer waiting with `None`, which the wait awaits before it goes on.
#[derive(Clone, Copy)]
pub struct JobWaiting<'a> {
    pub shown: &'a dyn Fn(Option<String>) -> LocalBoxFuture<'a, ()>,
}

/// Keeps what a command leaves when it ends, before its command reads as
/// ended: the copies of its edits (`crates/agent-store/src/media.rs`), and
/// its whole output (`crates/agent-server/src/server/shell_output.rs`). A failure to keep
/// them is the keeper's to record.
pub trait CommandKeeper {
    /// Records that `command` runs as the job `job` on the environment's Host,
    /// before the job starts, so that a backend that starts again takes it up,
    /// its reports name it by its title, and an interrupted call's command
    /// keeps whether it runs in the background (`crates/backend-database/src/command_outputs.rs`). A
    /// record of it as not yet sent becomes one of it as sent.
    ///
    /// A failure to record it is the keeper's to log: the command runs all the
    /// same.
    fn started<'a>(&'a self, command: &'a CommandId, job: &'a str, start: &'a KeptStart) -> LocalBoxFuture<'a, ()>;

    /// Records that `command` waits to start as the job `job` until its
    /// Host, a paired device whose runner is away, is back, with what it
    /// will send then, so a backend that starts again keeps it waiting and
    /// sends it once the runner connects (`crates/backend-host-access/src/access.rs`). A failure to record it is the keeper's to log.
    fn waiting<'a>(
        &'a self,
        command: &'a CommandId,
        job: &'a str,
        start: &'a KeptStart,
        unsent: &'a UnsentStart,
    ) -> LocalBoxFuture<'a, ()>;

    /// The list the command's view shows of `files`: every file, each
    /// segment with its copies when they were stored.
    fn retain<'a>(
        &'a self,
        command: &'a CommandId,
        files: &'a [JobFileChange],
    ) -> LocalBoxFuture<'a, Vec<EditedFile>>;

    /// The bytes of the blob `blob` when the conversation owner's
    /// namespace holds it, so a medium it holds is not read from the Host.
    fn stored_blob<'a>(&'a self, blob: &'a BlobRef) -> LocalBoxFuture<'a, Option<Bytes>>;

    /// Records that the model has seen `command`'s output as far as `seen`,
    /// in its record as running, so that a backend that starts again shows
    /// only what it has not seen
    /// (`crates/backend-database/src/command_outputs.rs`). A failure to
    /// record it is the keeper's to log.
    fn looked<'a>(&'a self, command: &'a CommandId, seen: Seen) -> LocalBoxFuture<'a, ()>;

    /// Records the command's end, `end`, with its whole output and the
    /// title of the call that started it, in place of its record as
    /// running.
    fn keep_output<'a>(
        &'a self,
        command: &'a CommandId,
        title: &'a str,
        end: CommandEnd,
        output: &'a WholeOutput,
    ) -> LocalBoxFuture<'a, ()>;
}

/// What a command's record keeps of the call that started it: the call
/// `caller` names, titled `title`, started it `background` or not.
pub struct KeptStart {
    pub caller: JobCaller,
    pub title: String,
    /// The call's script, which the record keeps with its title for the
    /// pages' view.
    pub script: String,
    pub background: bool,
}

/// Builds each job's command context (`crates/command-protocol/src/invocation.rs`).
pub type ContextSource = Rc<dyn Fn() -> LocalBoxFuture<'static, Result<CommandContext, HostError>>>;

/// What an environment runs with.
pub struct EnvironmentOptions {
    pub host: RemoteHost,
    /// The commands its jobs may run; none for jobs without declared
    /// commands.
    pub commands: Option<CommandSelection>,
    pub context: ContextSource,
    /// Where the pages' views of its commands go, and whether a page
    /// watches, which its jobs follow.
    pub feed: Rc<dyn PageFeed>,
    /// Where its commands' numbers come from: the conversation's
    /// sequences.
    pub numbers: Rc<dyn Numbers>,
    pub access: Option<Rc<dyn HostAccess>>,
    pub keeper: Option<Rc<dyn CommandKeeper>>,
    /// The variables every job starts with, above the device's own.
    pub initial_env: BTreeMap<String, String>,
    /// The budget of one status view's new output, per stream.
    pub output_limit: usize,
    /// The most of a binary final stdout a status carries.
    pub binary_limit: usize,
}

impl EnvironmentOptions {
    pub fn new(
        host: RemoteHost,
        context: ContextSource,
        feed: Rc<dyn PageFeed>,
        numbers: Rc<dyn Numbers>,
    ) -> Self {
        Self {
            host,
            commands: None,
            context,
            feed,
            numbers,
            access: None,
            keeper: None,
            initial_env: BTreeMap::new(),
            output_limit: DEFAULT_OUTPUT_LIMIT_BYTES,
            binary_limit: DEFAULT_BINARY_LIMIT_BYTES,
        }
    }
}

/// Makes each node's environments against the catalog fixed at startup.
pub struct RemoteShellEnvironmentFactory {
    catalog: CommandCatalog,
}

impl RemoteShellEnvironmentFactory {
    pub fn new(catalog: CommandCatalog) -> Self {
        Self { catalog }
    }

    /// An environment whose jobs run `commands` on `options.host`.
    pub fn create(
        &self,
        commands: &CommandSet,
        mut options: EnvironmentOptions,
    ) -> Result<RemoteShellEnvironment, ManifestError> {
        options.commands = Some(self.catalog.select(commands)?);
        Ok(RemoteShellEnvironment::new(options))
    }
}

/// A node's commands on a Host behind a runner.
#[derive(Clone)]
pub struct RemoteShellEnvironment(Rc<Environment>);

struct Environment {
    /// What its jobs start with; `commands` replaces its commands.
    options: EnvironmentOptions,
    /// The manifest of its commands now, which each job pins as it starts:
    /// `options.commands` until a replacement.
    commands: RefCell<Option<CommandSelection>>,
    state: RefCell<State>,
    /// The jobs' tasks.
    tasks: TaskTracker,
    /// Cancelled when the environment lets go of its commands, which run
    /// on.
    detached: CancellationToken,
    /// Changes each time one of its commands ends, which a wait for a
    /// command's end watches.
    ends: watch::Sender<()>,
}

#[derive(Default)]
struct State {
    records: HashMap<CommandId, Rc<RefCell<CommandRecord>>>,
    running: HashMap<CommandId, Rc<Running>>,
}

/// How a command's run begins: with a new job, or with one it takes up
/// again.
enum Begin {
    /// A new job `job`, whose record as not yet sent exists already when
    /// `recorded`, as for one a backend that started again sends.
    Start { request: ExecRequest, job: String, recorded: bool },
    Adopt { job: String, caller: JobCaller },
}

/// How following a job ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Followed {
    /// The command settled, or never ran.
    Ended,
    /// The environment let go of it, and it runs on.
    Detached,
}

/// How a command ended and the whole output it settles with, which
/// [`CommandRecord::settle`] takes and the keeper records.
struct Settlement {
    end: CommandEnd,
    output: WholeOutput,
    binary_stdout: Option<BinaryOutput>,
    media: Vec<CommandMedium>,
    /// What the end adds to the pages' view.
    page: String,
}

/// A job that ended: the job, its end as the runner reported it, and what
/// the backend received of each of its streams.
struct EndedJob<'a> {
    job: &'a RemoteJob,
    end: JobEnd,
    streams: [Received; 2],
}

/// A command that has not ended.
struct Running {
    /// Stops it: an abort, or the caller's cancellation.
    stop: CancellationToken,
    /// Its job, once started.
    job: watch::Sender<Option<RemoteJob>>,
    /// The output the backend received within the streams' views, in the
    /// order it came: the whole output when neither stream went beyond.
    received: RefCell<Vec<OutputRecord>>,
    settled: watch::Sender<bool>,
}

impl Running {
    async fn settled(&self) {
        let mut settled = self.settled.subscribe();
        // The sender lives as long as this `Running`.
        let _ = settled.wait_for(|settled| *settled).await;
    }

    /// Its job, once it started: a command still acquiring its Host, such as
    /// a Cloud that wakes, is waited for; none when it ended without one.
    async fn started(&self) -> Option<RemoteJob> {
        let mut job = self.job.subscribe();
        tokio::select! {
            // The sender lives as long as this `Running`.
            started = job.wait_for(Option::is_some) => started.ok().and_then(|job| job.clone()),
            () = self.settled() => self.job.borrow().clone(),
        }
    }
}

impl RemoteShellEnvironment {
    pub fn new(options: EnvironmentOptions) -> Self {
        Self(Rc::new(Environment {
            commands: RefCell::new(options.commands.clone()),
            options,
            state: RefCell::default(),
            tasks: TaskTracker::new(),
            detached: CancellationToken::new(),
            ends: watch::Sender::new(()),
        }))
    }

    /// The manifest of the commands a job that starts now offers.
    pub fn selection(&self) -> Option<CommandSelection> {
        self.0.commands.borrow().clone()
    }

    /// Starts the command, numbered with the conversation's next command
    /// number, as a job of its own.
    async fn start_command(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> Result<CommandId, ShellError> {
        let command = numbered(self.0.options.numbers.next(Sequence::Command).await?);
        let record = CommandRecord::new(
            command.clone(),
            request.caller.tool_use_id.clone(),
            request.title.clone(),
            &request.script,
        );
        let (running, record) = self.hold(&command, record, cancel.child_token());
        let environment = self.clone();
        let task_command = command.clone();
        let begin = Begin::Start {
            request,
            job: RemoteHost::job_id(),
            recorded: false,
        };
        self.0.tasks.spawn_local(async move {
            environment.run(task_command, begin, running, record).await;
        });
        Ok(command)
    }

    /// Takes up `taken`, which its call started on this environment's Host
    /// before the backend restarted or the environment that ran it let go of it
    /// (`crates/backend-remote-host/src/device_jobs.rs`).
    ///
    /// The command is followed to its end as if nothing had happened, counting
    /// its time from its start and showing the model only output it has not
    /// seen. Its runner's next hello says whether it went on, ended or was
    /// lost. One whose start was never sent waits for its Host again and is
    /// sent once its runner connects
    /// (`crates/backend-host-access/src/access.rs`). A command the environment
    /// holds already stays as it is.
    pub fn adopt(&self, taken: TakenUp) {
        let TakenUp {
            command,
            job,
            caller,
            title,
            script,
            background,
            running,
            seen,
            unsent,
        } = taken;
        if self.0.state.borrow().records.contains_key(&command) {
            return;
        }
        let record = CommandRecord::taken_up(
            command.clone(),
            caller.tool_use_id.clone(),
            title.clone(),
            &script,
            running,
            seen,
        );
        let (running, record) = self.hold(&command, record, CancellationToken::new());
        let begin = match unsent {
            Some(UnsentStart { script, cwd, viewable }) => Begin::Start {
                request: ExecRequest {
                    script,
                    cwd,
                    caller,
                    title,
                    background,
                    viewable,
                },
                job,
                recorded: true,
            },
            None => Begin::Adopt { job, caller },
        };
        let environment = self.clone();
        self.0.tasks.spawn_local(async move {
            environment.run(command, begin, running, record).await;
        });
    }

    /// Holds a new running command and tells the pages of it.
    fn hold(
        &self,
        command: &CommandId,
        record: CommandRecord,
        stop: CancellationToken,
    ) -> (Rc<Running>, Rc<RefCell<CommandRecord>>) {
        let record = Rc::new(RefCell::new(record));
        let running = Rc::new(Running {
            stop,
            job: watch::Sender::new(None),
            received: RefCell::new(Vec::new()),
            settled: watch::Sender::new(false),
        });
        {
            let mut state = self.0.state.borrow_mut();
            state.records.insert(command.clone(), record.clone());
            state.running.insert(command.clone(), running.clone());
        }
        // The pages learn of the command before its first output.
        self.report(&record);
        (running, record)
    }

    /// Tells the pages that `record`'s view changed.
    fn report(&self, record: &Rc<RefCell<CommandRecord>>) {
        self.0.options.feed.changed(record);
    }

    /// Runs one command to its end, inside the product's host access when there
    /// is one. A command taken up again runs outside it, since it holds its
    /// Host already and its runner may still be away. A command the environment
    /// lets go of runs on, unsettled, and so does one whose start still waits
    /// for its Host, which its record keeps waiting.
    async fn run(
        &self,
        command: CommandId,
        begin: Begin,
        running: Rc<Running>,
        record: Rc<RefCell<CommandRecord>>,
    ) {
        let failure: RefCell<Option<String>> = RefCell::new(None);
        let followed = Cell::new(Followed::Ended);
        let mut wait = None;
        let job = match begin {
            Begin::Start { request, job, recorded } => {
                let start = KeptStart {
                    caller: request.caller.clone(),
                    title: request.title.clone(),
                    script: request.script.clone(),
                    background: request.background,
                };
                let unsent = UnsentStart {
                    script: request.script.clone(),
                    cwd: request.cwd.clone(),
                    viewable: request.viewable.clone(),
                };
                wait = Some((job.clone(), start, unsent, Cell::new(recorded)));
                Begin::Start { request, job, recorded }
            }
            adopt @ Begin::Adopt { .. } => adopt,
        };
        let adopted = matches!(job, Begin::Adopt { .. });
        let job = async {
            let ran = match job {
                Begin::Start { request, job, .. } => {
                    let (_, start, _, _) = wait.as_ref().expect("a new job has its start");
                    self.execute(&command, request, &job, start, &running, &record).await
                }
                Begin::Adopt { job, caller } => {
                    let job = self.0.options.host.adopt_job(
                        job,
                        self.context().await,
                        Some(caller),
                        self.selection(),
                        running.stop.clone(),
                    );
                    // The runner's following of the job is unknown.
                    Ok(self.follow(&command, job, false, &running, &record).await)
                }
            };
            match ran {
                Ok(outcome) => followed.set(outcome),
                Err(error) => *failure.borrow_mut() = Some(error),
            }
        };
        // The pages show a command whose start waits for its Host as
        // waiting for it, and its record keeps it, not yet sent, from the
        // wait's start.
        let shown = |host: Option<String>| -> LocalBoxFuture<'_, ()> {
            Box::pin(async {
                let waits = host.is_some();
                if record.borrow_mut().mark_waiting(host) {
                    self.report(&record);
                }
                if let (true, Some((job, start, unsent, recorded)), Some(keeper)) =
                    (waits, &wait, &self.0.options.keeper)
                    && !recorded.replace(true)
                {
                    keeper.waiting(&command, job, start, unsent).await;
                }
            })
        };
        // A wait for the Host ends when the command stops, and when the
        // environment lets go of its commands, which leaves it waiting in
        // its record.
        let admission = running.stop.child_token();
        let access = match (&self.0.options.access, &wait) {
            (Some(access), Some(_)) if !adopted => {
                let key = self.0.options.host.key();
                let waiting = JobWaiting { shown: &shown };
                let ran = access.run_job(&key, admission.clone(), waiting, Box::pin(job));
                let let_go = async {
                    self.0.detached.cancelled().await;
                    admission.cancel();
                    std::future::pending::<()>().await;
                };
                tokio::select! {
                    ran = ran => ran.map_err(|error| error.message),
                    () = let_go => unreachable!("the wait for a let-go never ends"),
                }
            }
            _ => {
                job.await;
                Ok(())
            }
        };
        if followed.get() == Followed::Detached {
            return;
        }
        // Let go of before its job started: it never ran, and its record
        // keeps it for the backend that takes it up again.
        if self.0.detached.is_cancelled() && !running.stop.is_cancelled() && running.job.borrow().is_none() {
            return;
        }
        let failure = access.err().or_else(|| failure.into_inner());
        if let Some(reason) = failure
            && record.borrow().is_running()
        {
            let mut output = running.received.take();
            let (end, page) = if running.stop.is_cancelled() {
                (CommandEnd::Stopped, String::new())
            } else {
                (CommandEnd::Exited { exit_code: 127 }, push_reason(&mut output, &reason))
            };
            let output = WholeOutput::new(output, None);
            let settlement = Settlement {
                end,
                output,
                binary_stdout: None,
                media: Vec::new(),
                page,
            };
            self.end(&command, &record, settlement, None).await;
        }
        self.0.state.borrow_mut().running.remove(&command);
        running.settled.send_replace(true);
    }

    /// The command context of a job taken up again, whose rpc calls act
    /// for its node; with none, its calls are refused.
    async fn context(&self) -> Option<CommandContext> {
        match (self.0.options.context)().await {
            Ok(context) => Some(context),
            Err(error) => {
                tracing::warn!("a command taken up again has no command context: {error}");
                None
            }
        }
    }

    /// Starts the job and follows it to its end; an error is why it never
    /// ran.
    async fn execute(
        &self,
        command: &CommandId,
        request: ExecRequest,
        id: &str,
        start: &KeptStart,
        running: &Running,
        record: &Rc<RefCell<CommandRecord>>,
    ) -> Result<Followed, String> {
        let context = (self.0.options.context)()
            .await
            .map_err(|error| error.message)?;
        if running.stop.is_cancelled() {
            return Err("Shell command aborted".into());
        }
        // Every job starts in its node's working directory on the Host, the
        // conversation's for the root (`crates/agent-tools/src/lib.rs`).
        let cwd = request
            .cwd
            .unwrap_or_else(|| self.0.options.host.default_cwd().to_owned());
        let mut env = self.0.options.initial_env.clone();
        env.insert("PWD".into(), cwd.clone());
        if let Some(keeper) = &self.0.options.keeper {
            keeper.started(command, id, start).await;
        }
        let job = self
            .0
            .options
            .host
            .start_job(JobStart {
                id: id.to_owned(),
                script: request.script,
                cwd,
                env,
                context,
                viewable: Some(request.viewable),
                caller: Some(request.caller),
                commands: self.selection(),
                stdin: None,
                stdout: None,
                stop: running.stop.clone(),
            })
            .await
            .map_err(|error| error.message)?;
        // A new job starts unfollowed.
        Ok(self.follow(command, job, true, running, record).await)
    }

    /// Follows the job to its end and settles the command with it, unless
    /// the environment lets go of it first. `unfollowed` says the runner
    /// does not follow the job, as for one that just started; otherwise the
    /// runner is told first.
    async fn follow(
        &self,
        command: &CommandId,
        job: RemoteJob,
        unfollowed: bool,
        running: &Running,
        record: &Rc<RefCell<CommandRecord>>,
    ) -> Followed {
        running.job.send_replace(Some(job.clone()));
        let mut streams = [Received::default(), Received::default()];
        // The job is followed while a page watches; it starts unfollowed.
        let mut watching = self.0.options.feed.watching();
        let mut watch_open = true;
        let mut followed = false;
        // A new connection knows nothing of how the job was followed.
        let mut told = unfollowed;
        // A stop shows the command stopping until its runner says that the
        // job ended, which ends the output below.
        let stop = async {
            running.stop.cancelled().await;
            if record.borrow_mut().mark_stopping() {
                self.report(record);
            }
            job.stop().await;
        };
        tokio::pin!(stop);
        let mut stopped = false;
        let mut attachments = job.clone();
        loop {
            let follow = watch_open && *watching.borrow_and_update();
            if follow != followed || !told {
                followed = follow;
                told = true;
                if let Err(error) = job.follow(followed).await {
                    // The job's end or the connection's loss says what
                    // happened; a following that could not change is
                    // diagnostic.
                    tracing::debug!(%command, "could not change the job's following: {error}");
                }
            }
            tokio::select! {
                chunk = job.next_output() => {
                    let Some(chunk) = chunk else {
                        break;
                    };
                    let stream = &mut streams[stream_index(chunk.stream)];
                    if stream.receive(record, chunk, &running.received) {
                        self.report(record);
                    }
                }
                changed = watching.changed(), if watch_open => {
                    // A feed that is gone has no page.
                    watch_open = changed.is_ok();
                }
                () = attachments.reattached() => told = false,
                () = &mut stop, if !stopped => stopped = true,
                () = self.0.detached.cancelled() => {
                    job.let_go();
                    return Followed::Detached;
                }
            }
        }
        let end = job.end().await;
        let ended = EndedJob {
            job: &job,
            end,
            streams,
        };
        self.finish(command, running, record, ended).await;
        Followed::Ended
    }

    /// Settles the record from the job's end: its edits and its whole
    /// output, which the backend received or reads from the Host
    /// now; the runner then lets the job's directory go.
    async fn finish(
        &self,
        command: &CommandId,
        running: &Running,
        record: &Rc<RefCell<CommandRecord>>,
        ended: EndedJob<'_>,
    ) {
        let EndedJob { job, end, streams } = ended;
        // The reads the completion needs go out together (`crates/runner/src/environment.rs`): the edits' copies, the kept output and the
        // media.
        let retained = async {
            if end.files.is_empty() && end.path_changes.is_empty() {
                return None;
            }
            let files = match &self.0.options.keeper {
                Some(keeper) => keeper.retain(command, &end.files).await,
                None => end.files.iter().map(unkept).collect(),
            };
            Some(EditedFiles {
                files,
                path_changes: end.path_changes.iter().map(path_change).collect(),
                truncated: end.files_truncated,
            })
        };
        let mut received = running.received.take();
        let set_files = |files: Option<EditedFiles>| {
            if let Some(files) = files {
                record.borrow_mut().set_files(files);
            }
        };
        let exit_code = match &end.status {
            // A job the runner could not run names why; any other kind is
            // bash's own.
            ProcessEnd::NotStarted(error) => {
                let reason = match (&error.kind, &error.detail) {
                    (SpawnErrorKind::Other, Some(detail)) => detail.clone(),
                    _ => format!("bash: {}", error.kind),
                };
                let page = push_reason(&mut received, &reason);
                set_files(retained.await);
                let settlement = Settlement {
                    end: CommandEnd::Exited { exit_code: 127 },
                    output: WholeOutput::new(received, None),
                    binary_stdout: None,
                    media: Vec::new(),
                    page,
                };
                return self.end(command, record, settlement, Some(job)).await;
            }
            ProcessEnd::Lost(reason) => {
                // What the Host held beyond what the backend received went
                // with the connection. The output keeps only what the
                // command printed: the end says why it was lost
                // (`crates/agent-server/src/server/shell_output.rs`).
                let missing = Some(Missing {
                    bytes: streams.iter().map(Received::unreceived).sum(),
                    reason: LOST.into(),
                })
                .filter(|missing| missing.bytes > 0);
                let page = missing
                    .as_ref()
                    .map_or_else(String::new, |missing| format!("{}\n", missing.line()));
                set_files(retained.await);
                // The media went with the connection too.
                let media = job
                    .media()
                    .into_iter()
                    .map(|medium| command_medium(medium, Err(LOST.into())))
                    .collect();
                let settlement = Settlement {
                    end: CommandEnd::Lost {
                        reason: reason.clone(),
                    },
                    output: WholeOutput::new(received, missing),
                    binary_stdout: None,
                    media,
                    page,
                };
                return self.end(command, record, settlement, Some(job)).await;
            }
            ProcessEnd::Exited(code) => *code,
            ProcessEnd::Signalled(signal) if matches!(signal.as_str(), "SIGTERM" | "SIGKILL") => {
                130
            }
            ProcessEnd::Signalled(_) => 128,
        };
        let lengths = end.output.unwrap_or(wire::OutputLengths {
            stdout_bytes: streams[0].received(),
            stderr_bytes: streams[1].received(),
        });
        let unreceived = lengths.stdout_bytes.saturating_sub(streams[0].received())
            + lengths.stderr_bytes.saturating_sub(streams[1].received());
        // What the end adds to the pages' view: the whole output when the
        // view lacks some of it, which shows its end anew; otherwise only a
        // line that stands for a binary stdout.
        let kept_output = async {
            if unreceived == 0 {
                None
            } else {
                Some(job.read_output().await)
            }
        };
        let (files, kept_output, media) = tokio::join!(retained, kept_output, self.media(job));
        set_files(files);
        let (output, mut page, read) = match kept_output {
            None => (WholeOutput::new(received, None), String::new(), false),
            Some(Ok(output)) => (output, String::new(), true),
            Some(Err(error)) => {
                tracing::warn!(%command, "could not read the command's kept output: {error}");
                // The output ends with each stream's newest bytes the
                // runner sent, past what lies between them and the start.
                let newest = Received::newest_bytes(&streams);
                let kept: u64 = newest
                    .iter()
                    .map(|(left_out, _, bytes)| left_out + bytes.len() as u64)
                    .sum();
                if !newest.is_empty() {
                    let left_out = newest.iter().map(|(left_out, ..)| left_out).sum();
                    received.push(OutputRecord::LeftOut(left_out));
                    received.extend(
                        newest
                            .into_iter()
                            .map(|(_, stream, bytes)| OutputRecord::Output(stream, bytes)),
                    );
                }
                let missing = Some(Missing {
                    bytes: unreceived.saturating_sub(kept),
                    reason: format!("not read from the Host: {}", error.message),
                })
                .filter(|missing| missing.bytes > 0);
                let page = missing
                    .as_ref()
                    .map_or_else(String::new, |missing| format!("{}\n", missing.line()));
                (WholeOutput::new(received, missing), page, false)
            }
        };
        let binary = output.binary_stdout(lengths.stdout_bytes, self.0.options.binary_limit);
        let binary_length = binary.as_ref().map(|binary| binary.info.total_bytes);
        if read {
            page = output
                .text(Streams::Both, binary_length, Seen::default())
                .display();
        } else if let Some(length) = binary_length {
            page.insert_str(0, &format!("{}\n", binary_line(length)));
        }
        // A stop of the command, or of the call that watches it, is a stop
        // (`crates/agent-server/src/server/tree/live.rs`), whatever signal ended the job.
        let stopped = running.stop.is_cancelled();
        let end = if stopped {
            CommandEnd::Stopped
        } else {
            CommandEnd::Exited { exit_code }
        };
        {
            let mut record = record.borrow_mut();
            record.grew(StreamKind::Stdout, lengths.stdout_bytes);
            record.grew(StreamKind::Stderr, lengths.stderr_bytes);
        }
        let settlement = Settlement {
            end,
            output,
            binary_stdout: binary,
            media,
            page,
        };
        self.end(command, record, settlement, Some(job)).await;
    }

    /// The media the job's commands returned
    /// (`crates/agent-tools/src/result.rs`), each with its bytes: from the
    /// conversation owner's namespace when it holds them, otherwise read from
    /// the Host, all with one request; or why the backend does not have them.
    async fn media(&self, job: &RemoteJob) -> Vec<CommandMedium> {
        let announced = job.media();
        let mut held = Vec::with_capacity(announced.len());
        for medium in &announced {
            held.push(match &self.0.options.keeper {
                Some(keeper) => keeper.stored_blob(&medium.sha256).await,
                None => None,
            });
        }
        let unheld: Vec<u32> = announced
            .iter()
            .zip(&held)
            .filter(|(_, held)| held.is_none())
            .map(|(medium, _)| medium.number)
            .collect();
        let mut read = if unheld.is_empty() {
            Vec::new()
        } else {
            match job.read_media(&unheld).await {
                Ok(read) => read,
                Err(error) => unheld.iter().map(|_| Err(error.clone())).collect(),
            }
        }
        .into_iter();
        announced
            .into_iter()
            .zip(held)
            .map(|(medium, held)| {
                let bytes = match held {
                    Some(bytes) => Ok(bytes),
                    None => match read.next().expect("one answer per medium read") {
                        Ok(bytes) if BlobRef::of(&bytes) == medium.sha256 => Ok(bytes),
                        Ok(_) => Err("not read from the Host: its bytes are not the ones the runner announced".into()),
                        Err(error) => Err(format!("not read from the Host: {}", error.message)),
                    },
                };
                command_medium(medium, bytes)
            })
            .collect()
    }

    /// Ends the command with its whole output: the keeper stores it, the
    /// runner lets the job's directory go, since the backend now has what it
    /// needs of the job, and the record settles, the pages' view with
    /// `page` added.
    async fn end(
        &self,
        command: &CommandId,
        record: &Rc<RefCell<CommandRecord>>,
        settlement: Settlement,
        job: Option<&RemoteJob>,
    ) {
        let Settlement {
            end,
            output,
            binary_stdout,
            media,
            page,
        } = settlement;
        if let Some(keeper) = &self.0.options.keeper {
            let title = record.borrow().title().to_owned();
            keeper.keep_output(command, &title, end.clone(), &output).await;
        }
        if let Some(job) = job {
            job.release().await;
        }
        let ended =
            record
                .borrow_mut()
                .settle(ending_of(end), Arc::new(output), binary_stdout, media, &page);
        if ended {
            self.ended_now(record);
        }
    }

    /// `record`'s command ended just now: the pages and the waits for its
    /// end learn of it.
    fn ended_now(&self, record: &Rc<RefCell<CommandRecord>>) {
        self.report(record);
        self.0.ends.send_replace(());
    }

    fn record(&self, command: &CommandId) -> Result<Rc<RefCell<CommandRecord>>, ShellError> {
        self.0
            .state
            .borrow()
            .records
            .get(command)
            .cloned()
            .ok_or_else(|| ShellError::UnknownCommand(command.clone()))
    }

    /// The job of `command` while the command runs.
    fn running_job(&self, command: &CommandId) -> Result<Option<RemoteJob>, ShellError> {
        let record = self.record(command)?;
        if !record.borrow().is_running() {
            return Ok(None);
        }
        Ok(self
            .0
            .state
            .borrow()
            .running
            .get(command)
            .and_then(|running| running.job.borrow().clone()))
    }

    /// The model's status of `command`.
    fn view(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        let (hint, outliving) = match self.running_job(command)? {
            Some(job) => (job.running_hint(), job.outliving()),
            None => (None, Vec::new()),
        };
        let record = self.record(command)?;
        let mut record = record.borrow_mut();
        let before = record.seen();
        let status = record.status(self.0.options.output_limit, hint, outliving);
        let seen = record.seen();
        // The record of a command that runs keeps the model's place; one
        // that ended has no record as running any more.
        if seen != before
            && record.is_running()
            && let Some(keeper) = self.0.options.keeper.clone()
        {
            let command = command.clone();
            self.0.tasks.spawn_local(async move {
                keeper.looked(&command, seen).await;
            });
        }
        Ok(status)
    }

    /// Stops a running command: asks it to end, and its job's task ends it
    /// when it does not, and settles it once its runner says that the job
    /// ended, or it is lost (`crates/runner-shell/src/job.rs`).
    /// Returns once the stop is asked.
    fn stop_command(&self, command: &CommandId) -> Result<(), ShellError> {
        let record = self.record(command)?;
        let running = self.0.state.borrow().running.get(command).cloned();
        if let Some(running) = running.filter(|_| record.borrow().is_running()) {
            running.stop.cancel();
            // The pages show the stop at once, before the job's task takes it.
            if record.borrow_mut().mark_stopping() {
                self.report(&record);
            }
        }
        Ok(())
    }

    /// Waits until `running` has settled, or the environment let go of it,
    /// which leaves it unsettled.
    async fn settled_or_let_go(&self, running: &Running) {
        tokio::select! {
            () = running.settled() => {}
            () = self.0.detached.cancelled() => {}
        }
    }
}

impl ShellEnvironment for RemoteShellEnvironment {
    fn start(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'_, Result<CommandId, ShellError>> {
        Box::pin(self.start_command(request, cancel))
    }

    fn ended<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<Ending, ShellError>> {
        Box::pin(async move {
            // The record outlives its release, which may come before this
            // wait sees the end.
            let record = self.record(command)?;
            let mut ends = self.0.ends.subscribe();
            loop {
                if let Some(ending) = record.borrow().ending() {
                    return Ok(ending);
                }
                // The sender lives in the environment `self` borrows.
                let _ = ends.changed().await;
            }
        })
    }

    fn status(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        self.view(command)
    }

    fn title(&self, command: &CommandId) -> Option<String> {
        Some(self.record(command).ok()?.borrow().title().to_owned())
    }

    fn quiet(&self, command: &CommandId) -> Result<Duration, ShellError> {
        Ok(self.record(command)?.borrow().quiet())
    }

    fn outliving(&self, command: &CommandId) -> Vec<String> {
        // A command the environment does not hold names no task.
        self.running_job(command)
            .ok()
            .flatten()
            .map(|job| job.outliving())
            .unwrap_or_default()
    }

    fn awaits_user(&self, command: &CommandId) -> Option<watch::Receiver<bool>> {
        Some(self.running_job(command).ok().flatten()?.awaits_user())
    }

    fn unreachable(&self, command: &CommandId) -> Option<Unreachable> {
        let running = self.0.state.borrow().running.get(command).cloned()?;
        let away = running.job.borrow().as_ref()?.away()?;
        Some(Unreachable {
            away_ms: u64::try_from(away.as_millis()).unwrap_or(u64::MAX),
        })
    }

    fn media(&self, command: &CommandId) -> Result<Vec<CommandMedium>, ShellError> {
        Ok(self.record(command)?.borrow().media().to_vec())
    }

    fn read_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<WholeOutput, ShellError>> {
        Box::pin(async move {
            let record = self.record(command)?;
            let running = self.0.state.borrow().running.get(command).cloned();
            let Some(running) = running.filter(|_| record.borrow().is_running()) else {
                return Err(ShellError::NotRunning(command.clone()));
            };
            let job = running
                .started()
                .await
                .ok_or_else(|| ShellError::NotRunning(command.clone()))?;
            Ok(job.read_output().await?)
        })
    }

    fn write<'a>(
        &'a self,
        command: &'a CommandId,
        stdin: Bytes,
    ) -> LocalBoxFuture<'a, Result<(), ShellError>> {
        Box::pin(async move {
            let record = self.record(command)?;
            let running = self.0.state.borrow().running.get(command).cloned();
            let Some(running) = running.filter(|_| record.borrow().is_running()) else {
                return Err(ShellError::NotRunning(command.clone()));
            };
            if stdin.is_empty() {
                return Err(ShellError::EmptyStdin);
            }
            // Input typed while the command acquires its Host waits for it.
            let job = running
                .started()
                .await
                .ok_or_else(|| ShellError::NotRunning(command.clone()))?;
            job.write_stdin(stdin).await?;
            Ok(())
        })
    }

    fn abort<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<(), ShellError>> {
        Box::pin(std::future::ready(self.stop_command(command)))
    }

    fn page_views(&self) -> Vec<PageView> {
        self.0
            .state
            .borrow()
            .records
            .values()
            .map(|record| record.borrow().page_view())
            .collect()
    }

    fn release_command<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, bool> {
        Box::pin(async move {
            if self.record(command).is_err() {
                return false;
            }
            let running = self.0.state.borrow().running.get(command).cloned();
            if let Some(running) = running {
                running.stop.cancel();
                self.settled_or_let_go(&running).await;
            }
            // A command that ran keeps its output when its job ends, and
            // its Host lets the job's directory go then.
            self.0.state.borrow_mut().records.remove(command);
            true
        })
    }

    fn adopt(&self, taken: TakenUp) {
        RemoteShellEnvironment::adopt(self, taken);
    }

    fn stop_waiting(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
            let waiting: Vec<CommandId> = {
                let state = self.0.state.borrow();
                state
                    .running
                    .keys()
                    .filter(|command| {
                        state
                            .records
                            .get(*command)
                            .is_some_and(|record| record.borrow().is_waiting())
                    })
                    .cloned()
                    .collect()
            };
            for command in waiting {
                // The environment holds every command its state lists.
                let _ = self.stop_command(&command);
            }
        })
    }

    fn detach_all(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
            self.0.detached.cancel();
            self.0.tasks.close();
            self.0.tasks.wait().await;
        })
    }

    fn dispose_all(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
            let running: Vec<_> = self.0.state.borrow().running.values().cloned().collect();
            for running in &running {
                running.stop.cancel();
            }
            for running in &running {
                self.settled_or_let_go(running).await;
            }
        })
    }

    fn owns_command(&self, command: &CommandId) -> bool {
        self.0.state.borrow().records.contains_key(command)
    }

    /// An environment for jobs without declared commands stays so.
    fn replace_commands(&self, commands: &Rc<CommandSet>) -> Result<(), HostError> {
        let mut held = self.0.commands.borrow_mut();
        let Some(selection) = held.as_ref() else {
            return Ok(());
        };
        let replaced = selection
            .of(commands)
            .map_err(|error| HostError::new(HostErrorKind::Protocol, error.to_string()))?;
        *held = Some(replaced);
        Ok(())
    }
}

/// The identity the model knows a command by: its number.
fn numbered(number: u64) -> CommandId {
    CommandId::try_from(number.to_string()).expect("a number is a nonempty identity")
}

fn stream_index(stream: StreamKind) -> usize {
    match stream {
        StreamKind::Stdout => 0,
        StreamKind::Stderr => 1,
    }
}

/// How the command record shows `end`. An environment always knows how
/// its commands ended; a command whose end it did not know would read as
/// one that never ran, with exit code 127.
fn ending_of(end: CommandEnd) -> Ending {
    match end {
        CommandEnd::Exited { exit_code } => Ending::Exited(exit_code),
        CommandEnd::Stopped => Ending::Aborted,
        CommandEnd::Lost { reason } => Ending::Lost(reason),
        CommandEnd::Unrecorded => Ending::Exited(127),
    }
}

/// Why the backend does not have what a Host held when the connection to it
/// was lost.
const LOST: &str = "lost with the Host's connection";

/// A medium the runner announced, with its bytes or why there are none.
fn command_medium(medium: JobMedium, bytes: Result<Bytes, String>) -> CommandMedium {
    CommandMedium {
        number: medium.number,
        media_type: medium.media_type,
        size: medium.size,
        name: medium.name,
        width: medium.width,
        height: medium.height,
        duration_ms: medium.duration_ms,
        bytes,
    }
}

/// A changed file whose copies were not stored.
fn unkept(file: &JobFileChange) -> EditedFile {
    edited_file(file, |_| None)
}

/// The record a view lists for a file a job changed: its line counts and,
/// for each edit segment, the copies `copies` answers for its index.
pub fn edited_file(
    file: &JobFileChange,
    mut copies: impl FnMut(usize) -> Option<EditCopies>,
) -> EditedFile {
    EditedFile {
        from: None,
        path: file.path.clone(),
        kind: match file.kind {
            JobEditKind::Added => EditKind::Added,
            JobEditKind::Modified => EditKind::Modified,
        },
        added: u32::try_from(file.added).unwrap_or(u32::MAX),
        removed: u32::try_from(file.removed).unwrap_or(u32::MAX),
        outside: false,
        edits: file
            .edits
            .iter()
            .enumerate()
            .map(|(segment, recorded)| EditSegment {
                copies: copies(segment),
                binary: recorded.binary,
            })
            .collect(),
    }
}

/// A rename or a removal as the view lists it.
fn path_change(change: &wire::JobPathChange) -> PathChange {
    match change {
        wire::JobPathChange::Renamed { from, to } => PathChange::Renamed {
            from: from.clone(),
            to: to.clone(),
        },
        wire::JobPathChange::Removed { path } => PathChange::Removed { path: path.clone() },
    }
}

/// Adds why a command ended without running to its end to its output: a
/// line on stderr, after the output's last line. Returns the text it added.
fn push_reason(output: &mut Vec<OutputRecord>, reason: &str) -> String {
    let last = output.iter().rev().find_map(|record| match record {
        OutputRecord::Output(_, bytes) => bytes.last().copied(),
        OutputRecord::LeftOut(_) => None,
    });
    let separator = if last.is_none_or(|byte| byte == b'\n') {
        ""
    } else {
        "\n"
    };
    let text = format!("{separator}{reason}\n");
    output.push(OutputRecord::Output(
        StreamKind::Stderr,
        Bytes::from(text.clone()),
    ));
    text
}

/// What the backend received of one stream of a job (`crates/runner-jobs/src/tasks.rs`).
#[derive(Default)]
struct Received {
    /// Where the stream's part of the output it holds ends: its first bytes,
    /// within `JOB_VIEW_BYTES`, up to the first it did not receive, such as
    /// bytes the runner printed while no connection served that no read
    /// delivered again. Bytes after such a gap stay out of it, so the
    /// output never joins bytes across it, and the end reads the kept
    /// output.
    head: u64,
    /// The stream's length on the Host, as its messages told it.
    host: u64,
    /// Where the pages' view of the stream's next bytes starts.
    next: u64,
    text: Utf8Stream,
    /// The stream's newest bytes beyond its first `JOB_VIEW_BYTES`.
    newest: Newest,
}

impl Received {
    /// Adds one message of the job's output: a stream's first bytes to both
    /// views of `record` and to the `received` output, its newest bytes
    /// beyond them to the pages' view, and a message without bytes as
    /// growth alone. True when the pages' view changed.
    fn receive(
        &mut self,
        record: &RefCell<CommandRecord>,
        chunk: JobOutput,
        received: &RefCell<Vec<OutputRecord>>,
    ) -> bool {
        let end = chunk.offset + chunk.bytes.len() as u64;
        self.host = self.host.max(end);
        if chunk.offset < wire::JOB_VIEW_BYTES as u64 {
            // The device's jobs deliver a stream's first bytes without
            // overlap (`JobEntry::deliver`), so a message that does not
            // start where the held output ends lies past a gap.
            if chunk.offset == self.head {
                self.head = end;
                if !chunk.bytes.is_empty() {
                    received
                        .borrow_mut()
                        .push(OutputRecord::Output(chunk.stream, chunk.bytes.clone()));
                }
            }
            self.next = end;
            let text = self.text.decode(&chunk.bytes);
            return record.borrow_mut().append_output(chunk.stream, &text);
        }
        record.borrow_mut().grew(chunk.stream, end);
        if chunk.bytes.is_empty() {
            return false;
        }
        self.newest.keep(chunk.offset, &chunk.bytes);
        record.borrow_mut().set_newest(
            chunk.stream,
            self.newest.offset(),
            self.newest.offset().saturating_sub(self.head),
            whole_characters(self.newest.bytes()),
        );
        // The pages' view takes only the bytes it does not hold yet: a job
        // nobody follows sends its newest bytes again with each message.
        let held = usize::try_from(self.next.saturating_sub(chunk.offset)).unwrap_or(usize::MAX);
        let Some(fresh) = chunk.bytes.get(held..).filter(|fresh| !fresh.is_empty()) else {
            return false;
        };
        let mut text = String::new();
        // The runner left out what lies between the previous bytes and these.
        let left_out = chunk.offset.saturating_sub(self.next);
        if left_out > 0 {
            text.push_str(&self.text.finish());
            text.push_str(&left_out_note(chunk.stream, left_out));
        }
        text.push_str(&self.text.decode(fresh));
        self.next = end;
        record.borrow_mut().append_page_output(&text)
    }

    /// The bytes of the stream in the output the backend holds.
    fn received(&self) -> u64 {
        self.head
    }

    /// Each stream's newest bytes beyond its start, with the bytes left out
    /// between the start and them.
    fn newest_bytes(streams: &[Received]) -> Vec<(u64, StreamKind, Bytes)> {
        [StreamKind::Stdout, StreamKind::Stderr]
            .into_iter()
            .zip(streams)
            .filter(|(_, stream)| !stream.newest.is_empty())
            .map(|(kind, stream)| {
                (
                    stream.newest.offset().saturating_sub(stream.head),
                    kind,
                    Bytes::copy_from_slice(stream.newest.bytes()),
                )
            })
            .collect()
    }

    /// The bytes of the stream the Host held beyond those, as far as its
    /// messages told.
    fn unreceived(&self) -> u64 {
        self.host.saturating_sub(self.head)
    }
}

/// The text of a stream's newest bytes: a character cut at their start or
/// their end is left out, and other bytes that are not UTF-8 read as U+FFFD.
fn whole_characters(bytes: &[u8]) -> String {
    // A character's continuation bytes, which no character starts with.
    let start = bytes
        .iter()
        .take(3)
        .take_while(|byte| (**byte & 0b1100_0000) == 0b1000_0000)
        .count();
    let bytes = &bytes[start..];
    let end = match std::str::from_utf8(bytes) {
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        _ => bytes.len(),
    };
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// The pages' note where the runner left out `bytes` of a stream.
fn left_out_note(stream: StreamKind, bytes: u64) -> String {
    format!("\n[... {bytes} bytes of {stream} not shown ...]\n")
}

/// Decodes one stream's bytes as they arrive: a character split across
/// chunks waits for its rest, and bytes that are not UTF-8 read as U+FFFD,
/// as a lossy decode of the whole stream would. The standard library decodes
/// only whole buffers.
#[derive(Default)]
struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    fn decode(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        let mut rest = self.pending.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    text.push_str(valid);
                    rest = &[];
                    break;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    text.push_str(&String::from_utf8_lossy(valid));
                    match error.error_len() {
                        Some(length) => {
                            text.push(char::REPLACEMENT_CHARACTER);
                            rest = &after[length..];
                        }
                        // An incomplete character at the end waits for the
                        // next chunk.
                        None => {
                            rest = after;
                            break;
                        }
                    }
                }
            }
        }
        self.pending = rest.to_vec();
        text
    }

    /// The end of the bytes: a character they split reads as U+FFFD.
    fn finish(&mut self) -> String {
        let rest = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        rest
    }
}
