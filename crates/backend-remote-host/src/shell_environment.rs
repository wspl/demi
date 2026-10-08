//! `RemoteShellEnvironment`: the shell behind the `shell_*` tools on a Host
//! reached through its runner (`runner.md` § Shell jobs). Every exec is one
//! job; the record holds the model's view of its output, the head while it
//! runs and the whole output once it ended, and the pages' view, which also
//! holds the newest output the runner sends while a page watches and the job
//! is therefore followed (`runtime.md` § Live output). At the job's end the
//! product's keeper stores the whole output, and the runner lets the job's
//! directory go (`runtime.md` § The whole output). The directory a script
//! ends in carries into the shell's next exec; nothing else of the shell's
//! state does.

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
    DefaultShell,
    DEFAULT_OUTPUT_LIMIT_BYTES, EditedFiles, Ending, ExecRequest, Host, HostError, HostKey,
    JobCaller, Missing, Numbers, OutputRecord, PageFeed, PageView, ProcessEnd, Seen,
    ShellEnvironment, ShellError, ShellTarget, SpawnErrorKind, Streams, WholeOutput, binary_line,
};
use demi_runner_protocol::{
    manifest::ManifestError,
    wire::{self, JobFileChange},
};
use demi_shared_types::{
    BlobRef, CommandEnd, CommandId, EditCopies, EditKind, EditSegment, EditedFile, Sequence, ShellId,
    StreamKind,
};
use futures_util::future::LocalBoxFuture;
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    CommandCatalog, JobEnd, JobMedium, JobOutput, JobStart, RemoteHost, RemoteJob,
    manifest::CommandSelection,
};

/// How long an abort waits for the runner to report the job's end after
/// each signal.
const ABORT_GRACE: Duration = Duration::from_secs(5);

/// The product's host access around one job: it holds what the Host needs,
/// such as the conversation's file gate and a Cloud's admission, through the
/// job's start, output and edit publication, and refuses when the
/// conversation's Host is no longer `host`.
pub trait HostAccess {
    fn run_job<'a>(
        &'a self,
        host: &'a HostKey,
        cancel: CancellationToken,
        job: LocalBoxFuture<'a, ()>,
    ) -> LocalBoxFuture<'a, Result<(), HostError>>;
}

/// Keeps what a command leaves when it ends, before its command reads as
/// ended: the copies of its edits (`edit-tracking.md` § Edit copies), and
/// its whole output with its media (`runtime.md` § The whole output, § Where
/// media are kept). A failure to keep them is the keeper's to record.
pub trait CommandKeeper {
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

    /// Records the command's end, `end`, with its whole output and media.
    fn keep_output<'a>(
        &'a self,
        command: &'a CommandId,
        end: CommandEnd,
        output: &'a WholeOutput,
        media: &'a [CommandMedium],
    ) -> LocalBoxFuture<'a, ()>;
}

/// Builds each job's command context (`native-runtime.md` § Command
/// context).
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
    /// Where its commands' and shells' numbers come from: the
    /// conversation's sequences.
    pub numbers: Rc<dyn Numbers>,
    pub access: Option<Rc<dyn HostAccess>>,
    pub keeper: Option<Rc<dyn CommandKeeper>>,
    /// The variables every shell starts with, above the device's own.
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

/// A node's shells on a Host behind a runner.
#[derive(Clone)]
pub struct RemoteShellEnvironment(Rc<Environment>);

struct Environment {
    options: EnvironmentOptions,
    state: RefCell<State>,
    /// The jobs' tasks.
    tasks: TaskTracker,
    /// Changes each time one of its commands ends, which a wait for a
    /// command's end watches.
    ends: watch::Sender<()>,
}

#[derive(Default)]
struct State {
    shells: HashMap<ShellId, Shell>,
    default_shell: Option<ShellId>,
    /// A shell number taken for a shell not made yet.
    spare_shell: Option<ShellId>,
    records: HashMap<CommandId, Rc<RefCell<CommandRecord>>>,
    running: HashMap<CommandId, Rc<Running>>,
}

struct Shell {
    cwd: String,
    env: BTreeMap<String, String>,
    foreground: Option<CommandId>,
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
    /// Set by an abort: the end that follows is the stop, not the
    /// command's own.
    aborted: Cell<bool>,
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

    /// Waits for the command to settle, at most `within`; true when it did.
    async fn settled_within(&self, within: Duration) -> bool {
        tokio::time::timeout(within, self.settled()).await.is_ok()
    }
}

impl RemoteShellEnvironment {
    pub fn new(options: EnvironmentOptions) -> Self {
        Self(Rc::new(Environment {
            options,
            state: RefCell::default(),
            tasks: TaskTracker::new(),
            ends: watch::Sender::new(()),
        }))
    }

    /// Starts the command: it takes the conversation's next command number,
    /// then picks the exec's shell and reserves it in one step that nothing
    /// awaits in, so two execs never share a shell. A new shell takes the
    /// conversation's next shell number, fetched first when none is spare.
    /// An exec refused before its number is taken takes none.
    async fn start_command(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> Result<CommandId, ShellError> {
        if let ShellTarget::Existing(id) = &request.shell {
            check_free(&self.0.state.borrow(), id)?;
        }
        let command = numbered::<CommandId>(self.0.options.numbers.next(Sequence::Command).await?);
        let shell_id = loop {
            if let Some(shell) = self.reserve(&request.shell, &command)? {
                break shell;
            }
            let spare = numbered::<ShellId>(self.0.options.numbers.next(Sequence::Shell).await?);
            self.0.state.borrow_mut().spare_shell = Some(spare);
        };
        let record = Rc::new(RefCell::new(CommandRecord::new(
            shell_id.clone(),
            command.clone(),
            request.tool_use_id,
        )));
        let running = Rc::new(Running {
            stop: cancel.child_token(),
            job: watch::Sender::new(None),
            received: RefCell::new(Vec::new()),
            aborted: Cell::new(false),
            settled: watch::Sender::new(false),
        });
        {
            let mut state = self.0.state.borrow_mut();
            state.records.insert(command.clone(), record.clone());
            state.running.insert(command.clone(), running.clone());
        }
        // The pages learn of the command before its first output.
        self.report(&record);
        let environment = self.clone();
        let task_command = command.clone();
        self.0.tasks.spawn_local(async move {
            environment
                .run(
                    shell_id,
                    task_command,
                    request.script,
                    request.caller,
                    running,
                    record,
                )
                .await;
        });
        Ok(command)
    }

    /// Picks `target`'s shell and makes `command` its foreground; none when
    /// the pick needs a new shell and no shell number is spare. The shell
    /// it picks runs nothing: an existing one is checked again, since the
    /// state may have changed while the command's number was fetched.
    fn reserve(
        &self,
        target: &ShellTarget,
        command: &CommandId,
    ) -> Result<Option<ShellId>, ShellError> {
        let mut state = self.0.state.borrow_mut();
        let shell_id = match target {
            ShellTarget::Existing(id) => {
                check_free(&state, id)?;
                id.clone()
            }
            ShellTarget::New { cwd } => match self.create_shell(&mut state, cwd.clone()) {
                Some(id) => id,
                None => return Ok(None),
            },
            ShellTarget::Default => match state.default_shell.clone() {
                Some(id) if state.shells[&id].foreground.is_none() => id,
                busy => {
                    // A shell beside the busy default one starts where it is.
                    let cwd = busy.as_ref().map(|id| state.shells[id].cwd.clone());
                    let Some(id) = self.create_shell(&mut state, cwd) else {
                        return Ok(None);
                    };
                    if busy.is_none() {
                        state.default_shell = Some(id.clone());
                    }
                    id
                }
            },
        };
        state
            .shells
            .get_mut(&shell_id)
            .expect("the shell was just found")
            .foreground = Some(command.clone());
        Ok(Some(shell_id))
    }

    /// Makes a shell with the spare shell number, which starts in `cwd`, the
    /// Host's default when none, with the variables every shell starts
    /// with: no shell's variables carry from one exec to the next. None
    /// when no number is spare.
    fn create_shell(&self, state: &mut State, cwd: Option<String>) -> Option<ShellId> {
        let id = state.spare_shell.take()?;
        state.shells.insert(
            id.clone(),
            Shell {
                cwd: cwd.unwrap_or_else(|| self.0.options.host.default_cwd().to_owned()),
                env: self.0.options.initial_env.clone(),
                foreground: None,
            },
        );
        Some(id)
    }

    /// Tells the pages that `record`'s view changed.
    fn report(&self, record: &Rc<RefCell<CommandRecord>>) {
        self.0.options.feed.changed(record);
    }

    /// Runs one command to its end, inside the product's host access when
    /// there is one.
    async fn run(
        &self,
        shell: ShellId,
        command: CommandId,
        script: String,
        caller: JobCaller,
        running: Rc<Running>,
        record: Rc<RefCell<CommandRecord>>,
    ) {
        let failure: RefCell<Option<String>> = RefCell::new(None);
        let job = async {
            if let Err(error) = self
                .execute(&shell, &command, script, caller, &running, &record)
                .await
            {
                *failure.borrow_mut() = Some(error);
            }
        };
        let access = match &self.0.options.access {
            Some(access) => {
                let key = self.0.options.host.key();
                access
                    .run_job(&key, running.stop.clone(), Box::pin(job))
                    .await
                    .map_err(|error| error.message)
            }
            None => {
                job.await;
                Ok(())
            }
        };
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
        let mut state = self.0.state.borrow_mut();
        state.running.remove(&command);
        if let Some(shell) = state.shells.get_mut(&shell)
            && shell.foreground.as_ref() == Some(&command)
        {
            shell.foreground = None;
        }
        drop(state);
        running.settled.send_replace(true);
    }

    /// Starts the job and follows it to its end; an error is why it never
    /// ran.
    async fn execute(
        &self,
        shell: &ShellId,
        command: &CommandId,
        script: String,
        caller: JobCaller,
        running: &Running,
        record: &Rc<RefCell<CommandRecord>>,
    ) -> Result<(), String> {
        let context = (self.0.options.context)()
            .await
            .map_err(|error| error.message)?;
        if running.stop.is_cancelled() {
            return Err("Shell command aborted".into());
        }
        let (cwd, env) = {
            let state = self.0.state.borrow();
            let shell = &state.shells[shell];
            let mut env = shell.env.clone();
            env.insert("PWD".into(), shell.cwd.clone());
            (shell.cwd.clone(), env)
        };
        let job = self
            .0
            .options
            .host
            .start_job(JobStart {
                script,
                cwd,
                env,
                context,
                caller: Some(caller),
                commands: self.0.options.commands.clone(),
                stdin: None,
                stdout: None,
            })
            .await
            .map_err(|error| error.message)?;
        running.job.send_replace(Some(job.clone()));
        let mut streams = [Received::default(), Received::default()];
        // The job is followed while a page watches; it starts unfollowed.
        let mut watching = self.0.options.feed.watching();
        let mut watch_open = true;
        let mut followed = false;
        let stopped = running.stop.cancelled();
        tokio::pin!(stopped);
        let mut signalled = false;
        loop {
            let follow = watch_open && *watching.borrow_and_update();
            if follow != followed {
                followed = follow;
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
                () = &mut stopped, if !signalled => {
                    signalled = true;
                    if let Err(error) = job.kill(wire::Signal::Terminate).await {
                        // The job's end or the connection's loss says what
                        // happened; a signal that could not go is diagnostic.
                        tracing::debug!(%command, "could not interrupt the job: {error}");
                    }
                }
            }
        }
        let end = job.end().await;
        let ended = EndedJob {
            job: &job,
            end,
            streams,
        };
        self.finish(shell, command, running, record, ended).await;
        Ok(())
    }

    /// Settles the record from the job's end: its edits, its directory, and
    /// its whole output, which the backend received or reads from the Host
    /// now; the runner then lets the job's directory go.
    async fn finish(
        &self,
        shell: &ShellId,
        command: &CommandId,
        running: &Running,
        record: &Rc<RefCell<CommandRecord>>,
        ended: EndedJob<'_>,
    ) {
        let EndedJob { job, end, streams } = ended;
        // The reads the completion needs go out together (`runner.md`
        // § Host operations): the edits' copies, the kept output and the
        // media.
        let retained = async {
            if end.files.is_empty() {
                return None;
            }
            let files = match &self.0.options.keeper {
                Some(keeper) => keeper.retain(command, &end.files).await,
                None => end.files.iter().map(unkept).collect(),
            };
            Some(EditedFiles {
                files,
                truncated: end.files_truncated,
            })
        };
        if let Some(cwd) = end.cwd.clone()
            && let Some(shell) = self.0.state.borrow_mut().shells.get_mut(shell)
        {
            shell.cwd = cwd;
        }
        let mut received = running.received.take();
        record.borrow_mut().set_presented(end.presented.clone());
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
                // with the connection.
                let missing = Some(Missing {
                    bytes: streams.iter().map(Received::unreceived).sum(),
                    reason: LOST.into(),
                })
                .filter(|missing| missing.bytes > 0);
                let mut page = push_reason(&mut received, reason);
                if let Some(missing) = &missing {
                    page.push_str(&missing.line());
                    page.push('\n');
                }
                set_files(retained.await);
                // The media went with the connection too.
                let media = job
                    .media()
                    .into_iter()
                    .map(|medium| command_medium(medium, Err(LOST.into())))
                    .collect();
                let settlement = Settlement {
                    end: CommandEnd::Lost,
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
        let end = if running.aborted.get() {
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

    /// The media the job's commands returned (`runtime.md` § Where media
    /// are kept), each with its bytes: from the conversation owner's
    /// namespace when it holds them, otherwise read from the Host, all with
    /// one request; or why the backend does not have them.
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
            keeper.keep_output(command, end, &output, &media).await;
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

    /// The model's status of `command`.
    fn view(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        let record = self.record(command)?;
        let hint = self
            .0
            .state
            .borrow()
            .running
            .get(command)
            .and_then(|running| {
                running
                    .job
                    .borrow()
                    .as_ref()
                    .and_then(RemoteJob::running_hint)
            });
        let mut record = record.borrow_mut();
        let hint = if record.is_running() { hint } else { None };
        Ok(record.status(self.0.options.output_limit, hint))
    }

    /// Stops a running command: asks it to end, and ends it when it does not.
    async fn stop_command(&self, command: &CommandId) -> Result<(), ShellError> {
        let record = self.record(command)?;
        let running = self.0.state.borrow().running.get(command).cloned();
        let Some(running) = running.filter(|_| record.borrow().is_running()) else {
            return Ok(());
        };
        running.aborted.set(true);
        // The job's task asks the job to end.
        running.stop.cancel();
        if !running.settled_within(ABORT_GRACE).await {
            let job = running.job.borrow().clone();
            if let Some(job) = job
                && let Err(error) = job.kill(wire::Signal::Kill).await
            {
                tracing::debug!(%command, "could not kill the job: {error}");
            }
            if !running.settled_within(ABORT_GRACE).await {
                let ended = record.borrow_mut().mark_aborted();
                if ended {
                    self.ended_now(&record);
                }
            }
        }
        Ok(())
    }

    async fn dispose(&self, shell: &ShellId) -> bool {
        let foreground = match self.0.state.borrow().shells.get(shell) {
            None => return false,
            Some(shell) => shell.foreground.clone(),
        };
        if let Some(command) = foreground
            && let Err(error) = self.stop_command(&command).await
        {
            tracing::debug!(%command, "could not abort the shell's command: {error}");
        }
        let mut state = self.0.state.borrow_mut();
        state.shells.remove(shell);
        if state.default_shell.as_ref() == Some(shell) {
            state.default_shell = None;
        }
        true
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

    fn default_shell(&self) -> Option<DefaultShell> {
        let state = self.0.state.borrow();
        let id = state.default_shell.clone()?;
        let cwd = state.shells[&id].cwd.clone();
        Some(DefaultShell { id, cwd })
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

    fn read_medium<'a>(
        &'a self,
        command: &'a CommandId,
        number: u32,
    ) -> LocalBoxFuture<'a, Result<Bytes, ShellError>> {
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
            let returned = job.media();
            if !returned.iter().any(|medium| medium.number == number) {
                return Err(ShellError::NoMedium {
                    command: command.clone(),
                    number,
                    returned: returned.len(),
                });
            }
            let mut read = job.read_media(&[number]).await?;
            Ok(read.pop().expect("one answer per medium read")?)
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
        Box::pin(self.stop_command(command))
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
            let Ok(record) = self.record(command) else {
                return false;
            };
            let running = record.borrow().is_running();
            if running && let Err(error) = self.stop_command(command).await {
                tracing::debug!(%command, "could not abort the released command: {error}");
            }
            // A command that ran keeps its output when its job ends, and
            // its Host lets the job's directory go then.
            self.0.state.borrow_mut().records.remove(command);
            true
        })
    }

    fn dispose_shell<'a>(&'a self, shell: &'a ShellId) -> LocalBoxFuture<'a, bool> {
        Box::pin(self.dispose(shell))
    }

    fn dispose_all(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
            let shells: Vec<_> = self.0.state.borrow().shells.keys().cloned().collect();
            for shell in shells {
                self.dispose(&shell).await;
            }
            let running: Vec<_> = self.0.state.borrow().running.values().cloned().collect();
            for running in running {
                running.settled().await;
            }
        })
    }

    fn owns_shell(&self, shell: &ShellId) -> bool {
        self.0.state.borrow().shells.contains_key(shell)
    }

    fn owns_command(&self, command: &CommandId) -> bool {
        self.0.state.borrow().records.contains_key(command)
    }
}

/// Refuses `id` when the environment has no such shell or it runs a
/// command.
fn check_free(state: &State, id: &ShellId) -> Result<(), ShellError> {
    let shell = state
        .shells
        .get(id)
        .ok_or_else(|| ShellError::UnknownShell(id.clone()))?;
    match &shell.foreground {
        Some(command) => Err(ShellError::ShellBusy {
            shell: id.clone(),
            command: command.clone(),
        }),
        None => Ok(()),
    }
}

/// The identity the model knows a command or a shell by: its number.
fn numbered<T: TryFrom<String>>(number: u64) -> T
where
    T::Error: std::fmt::Debug,
{
    T::try_from(number.to_string()).expect("a number is a nonempty identity")
}

fn stream_index(stream: StreamKind) -> usize {
    match stream {
        StreamKind::Stdout => 0,
        StreamKind::Stderr => 1,
    }
}

/// How the command record shows `end`: one that ended with its Host's
/// connection reads as exit code 127, as one that never ran does.
fn ending_of(end: CommandEnd) -> Ending {
    match end {
        CommandEnd::Exited { exit_code } => Ending::Exited(exit_code),
        CommandEnd::Stopped => Ending::Aborted,
        CommandEnd::Lost | CommandEnd::Unrecorded => Ending::Exited(127),
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
        path: file.path.clone(),
        kind: match file.kind {
            JobEditKind::Added => EditKind::Added,
            JobEditKind::Modified => EditKind::Modified,
        },
        added: u32::try_from(file.added).unwrap_or(u32::MAX),
        removed: u32::try_from(file.removed).unwrap_or(u32::MAX),
        edits: (0..file.edits.len())
            .map(|segment| EditSegment {
                copies: copies(segment),
            })
            .collect(),
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

/// What the backend received of one stream of a job (`runner.md` § Pipes
/// and output).
#[derive(Default)]
struct Received {
    /// How many of the stream's first `JOB_VIEW_BYTES` it received: the
    /// stream's part of the output it holds.
    head: u64,
    /// The stream's length on the Host, as its messages told it.
    host: u64,
    /// Where the pages' view of the stream's next bytes starts.
    next: u64,
    text: Utf8Stream,
    /// The stream's newest bytes beyond its first `JOB_VIEW_BYTES`, at most
    /// `JOB_VIEW_BYTES` of them, and where they start.
    newest: Vec<u8>,
    newest_offset: u64,
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
            self.head = end;
            self.next = end;
            let text = self.text.decode(&chunk.bytes);
            if !chunk.bytes.is_empty() {
                received
                    .borrow_mut()
                    .push(OutputRecord::Output(chunk.stream, chunk.bytes));
            }
            return record.borrow_mut().append_output(chunk.stream, &text);
        }
        record.borrow_mut().grew(chunk.stream, end);
        if chunk.bytes.is_empty() {
            return false;
        }
        self.keep_newest(chunk.offset, &chunk.bytes);
        record.borrow_mut().set_newest(
            chunk.stream,
            self.newest_offset,
            self.newest_offset.saturating_sub(self.head),
            whole_characters(&self.newest),
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

    /// Adds `bytes` at `offset` to the stream's newest bytes: they continue
    /// or overlap the ones before, or replace them; the newest
    /// `JOB_VIEW_BYTES` stay.
    fn keep_newest(&mut self, offset: u64, bytes: &[u8]) {
        let end = self.newest_offset + self.newest.len() as u64;
        if self.newest.is_empty() || offset < self.newest_offset || offset > end {
            self.newest = bytes.to_vec();
            self.newest_offset = offset;
        } else {
            self.newest
                .truncate(usize::try_from(offset - self.newest_offset).expect("within the bytes"));
            self.newest.extend_from_slice(bytes);
        }
        let excess = self.newest.len().saturating_sub(wire::JOB_VIEW_BYTES);
        self.newest.drain(..excess);
        self.newest_offset += excess as u64;
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
                    stream.newest_offset.saturating_sub(stream.head),
                    kind,
                    Bytes::from(stream.newest.clone()),
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
