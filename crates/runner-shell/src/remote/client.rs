//! The runner's end of a job's shell: [`RemoteShell`] starts a shell process
//! per job, and [`RemoteJob`] is the job shell contract over its socket
//! (`mod.rs`).

use std::{
    collections::{BTreeSet, HashMap},
    io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use demi_runner_process::{
    command_client::ENDPOINT_ENV,
    job_shell::{JobShell, JobStart, ShellJob},
    lines,
    local_socket,
    process::{JOB_ID_ENV, OutputChunk, ProcessExit, ProcessInput},
};
use demi_runner_protocol::wire::{OutputStream, Signal};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    sync::{Semaphore, mpsc, oneshot, watch},
};
use tokio_util::sync::CancellationToken;

use super::wire::{self, Commands, Edits, Frame, FromShell, Start, ToShell};

/// How long a shell process may take to say where it listens: one that does
/// not within it is taken to have failed to start.
const LISTEN_PATIENCE: Duration = Duration::from_secs(30);

/// The chunks of input on their way to a job's shell, which the job has not
/// taken: input is paced by what the job reads, as a pipe paces it.
const INPUT_WINDOW: usize = 4;

/// Starts each job in a shell process of its own, running `program`, the
/// runner's own executable, and takes over the jobs a runner before it left.
pub struct RemoteShell {
    program: PathBuf,
    records: Records,
    /// Set once the runner leaves its jobs to the next.
    leaving: Arc<AtomicBool>,
    peers: Arc<Peers>,
}

impl RemoteShell {
    /// Starts shells of `program`, recording each in `records`, a directory
    /// the installation keeps from one runner to the next.
    pub fn new(program: PathBuf, records: PathBuf) -> Self {
        Self {
            program,
            records: Records(Arc::new(records)),
            leaving: Arc::default(),
            peers: Arc::default(),
        }
    }
}

/// The directory of the [`ShellRecord`]s, one per shell that runs, by its
/// job's id: a runner that starts next reaches each shell by its record.
#[derive(Clone)]
struct Records(Arc<PathBuf>);

impl Records {
    fn path(&self, job: &str) -> PathBuf {
        self.0.join(format!("{job}.json"))
    }

    async fn write(&self, record: &ShellRecord) -> io::Result<()> {
        tokio::fs::create_dir_all(&*self.0).await?;
        let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
        tokio::fs::write(self.path(&record.job), bytes).await
    }

    async fn read(&self, job: &str) -> io::Result<Option<ShellRecord>> {
        match tokio::fs::read(self.path(job)).await {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).map_err(io::Error::other)?)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The jobs of every record.
    async fn jobs(&self) -> Vec<String> {
        let Ok(mut entries) = tokio::fs::read_dir(&*self.0).await else {
            return Vec::new();
        };
        let mut jobs = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Some(job) = entry.file_name().to_str().and_then(|name| name.strip_suffix(".json")) {
                jobs.push(job.to_owned());
            }
        }
        jobs
    }

    fn remove(&self, job: &str) {
        let _ = std::fs::remove_file(self.path(job));
    }
}

/// How long the runner waits for the shells it asks to signal a task:
/// a shell of an earlier contract ignores the question.
const PEERS_PATIENCE: Duration = Duration::from_secs(2);

/// The connected shells, which one asks through the runner to signal a
/// background task another job started (`FromShell::SignalTask`).
#[derive(Default)]
struct Peers {
    sessions: Mutex<Vec<std::sync::Weak<Session>>>,
    next: AtomicU64,
    asks: Mutex<HashMap<u64, Ask>>,
}

/// A question the runner passed on, until a shell had the task or every
/// shell answered.
struct Ask {
    origin: std::sync::Weak<Session>,
    request: u64,
    waiting: usize,
}

impl Peers {
    fn live(&self) -> Vec<Arc<Session>> {
        self.sessions
            .lock()
            .expect("the sessions are intact")
            .iter()
            .filter_map(std::sync::Weak::upgrade)
            .collect()
    }

    fn session(&self, job: &str) -> Option<Arc<Session>> {
        self.live().into_iter().find(|session| session.job == job)
    }

    fn join(&self, session: &Arc<Session>) {
        let mut sessions = self.sessions.lock().expect("the sessions are intact");
        sessions.retain(|session| session.strong_count() > 0);
        sessions.push(Arc::downgrade(session));
    }

    /// Asks every shell but `origin`'s to signal `task`, and answers
    /// `origin`'s `request`.
    fn ask(self: &Arc<Self>, origin: &Arc<Session>, request: u64, task: i32, signal: i32) {
        let others: Vec<Arc<Session>> = self
            .live()
            .into_iter()
            .filter(|session| !Arc::ptr_eq(session, origin))
            .collect();
        if others.is_empty() {
            origin.send(&ToShell::TaskSignalled { request, found: false });
            return;
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        self.asks.lock().expect("the asks are intact").insert(
            id,
            Ask {
                origin: Arc::downgrade(origin),
                request,
                waiting: others.len(),
            },
        );
        for other in others {
            other.send(&ToShell::SignalTask {
                request: id,
                task,
                signal,
            });
        }
        let peers = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(PEERS_PATIENCE).await;
            peers.answer(id, false, true);
        });
    }

    /// A shell answered question `id`; `last` ends it whatever is left.
    fn answer(&self, id: u64, found: bool, last: bool) {
        let mut asks = self.asks.lock().expect("the asks are intact");
        let Some(ask) = asks.get_mut(&id) else { return };
        ask.waiting = ask.waiting.saturating_sub(1);
        if !(found || last || ask.waiting == 0) {
            return;
        }
        let ask = asks.remove(&id).expect("the ask is there");
        if let Some(origin) = ask.origin.upgrade() {
            origin.send(&ToShell::TaskSignalled {
                request: ask.request,
                found,
            });
        }
    }
}

impl JobShell for RemoteShell {
    fn start(&self, job: JobStart) -> BoxFuture<'_, io::Result<Box<dyn ShellJob>>> {
        Box::pin(async move {
            let job = RemoteJob::start(&self.program, job, self.shared()).await?;
            Ok(Box::new(job) as Box<dyn ShellJob>)
        })
    }

    fn builtin_names(&self) -> BTreeSet<String> {
        crate::interpreter::registrations().into_keys().collect()
    }

    fn reattach(
        &self,
        job: &str,
        lease: Option<Box<dyn Send + Sync>>,
    ) -> BoxFuture<'_, io::Result<Option<Box<dyn ShellJob>>>> {
        let job = job.to_owned();
        Box::pin(async move {
            Ok(RemoteJob::reattach(&job, lease, self.shared(), false)
                .await?
                .map(|job| Box::new(job) as Box<dyn ShellJob>))
        })
    }

    fn reattach_left(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let taken: std::collections::HashSet<String> =
                self.peers.live().into_iter().map(|session| session.job.clone()).collect();
            for job in self.records.jobs().await {
                if taken.contains(&job) {
                    continue;
                }
                match RemoteJob::reattach(&job, None, self.shared(), true).await {
                    // Its job has ended and been released: what it left
                    // running reaches the runner through it, and it goes
                    // once that ends.
                    Ok(Some(left)) => {
                        left.session.ended.store(true, Ordering::SeqCst);
                        left.session.send(&ToShell::ExitKept);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(event = "shell.reattach_failed", job_id = job.as_str(), %error, "a shell a runner before this one left was not taken over");
                        self.records.remove(&job);
                    }
                }
            }
        })
    }

    fn leave(&self) {
        self.leaving.store(true, Ordering::SeqCst);
    }

    /// The shell held the job's exit until the backend had it, so a runner
    /// that started again meanwhile still received it.
    fn released(&self, job: &str) {
        if let Some(session) = self.peers.session(job) {
            session.send(&ToShell::ExitKept);
        }
    }

    fn close(&self) {
        for session in self.peers.live() {
            session.send(&ToShell::End);
        }
    }
}

/// How a runner reaches a job's shell, which the runner that started it
/// records.
#[derive(Serialize, Deserialize)]
struct ShellRecord {
    contract: u32,
    job: String,
    endpoint: String,
}

/// What every job's shell shares with the runner's others.
#[derive(Clone)]
struct Shared {
    records: Records,
    leaving: Arc<AtomicBool>,
    peers: Arc<Peers>,
}

impl RemoteShell {
    fn shared(&self) -> Shared {
        Shared {
            records: self.records.clone(),
            leaving: self.leaving.clone(),
            peers: self.peers.clone(),
        }
    }
}

/// How long a runner waits for a shell a runner before it left to answer:
/// one that does not is taken to be gone with its job.
const REATTACH_PATIENCE: Duration = Duration::from_secs(5);

/// A job that runs in a shell process.
pub struct RemoteJob {
    input: mpsc::Sender<ProcessInput>,
    output: mpsc::Receiver<OutputChunk>,
    session: Arc<Session>,
    outliving: watch::Receiver<Vec<String>>,
    exit: watch::Receiver<Option<ProcessExit>>,
}

/// What the job's handle and its connection's tasks share.
struct Session {
    frames: mpsc::UnboundedSender<Bytes>,
    drains: Mutex<HashMap<u64, oneshot::Sender<()>>>,
    next_drain: AtomicU64,
    cancelled: AtomicBool,
    ended: AtomicBool,
    kept: Mutex<[u64; 2]>,
    /// The job whose shell this is.
    job: String,
    /// Cancelled once the shell's connection has ended.
    closed: CancellationToken,
    /// The runner's: once set, nothing cancels the job from this runner.
    leaving: Arc<AtomicBool>,
    peers: Arc<Peers>,
}

impl Session {
    fn send(&self, message: &ToShell) {
        if let Ok(frame) = wire::control(message) {
            let _ = self.frames.send(frame);
        }
    }

    fn cancel(&self) {
        if self.leaving.load(Ordering::SeqCst) {
            return;
        }
        if !self.cancelled.swap(true, Ordering::SeqCst) {
            self.send(&ToShell::Cancel);
        }
    }
}

impl RemoteJob {
    async fn start(program: &Path, job: JobStart, shared: Shared) -> io::Result<Self> {
        let JobStart {
            script,
            cwd,
            env,
            live,
            cancellation,
            commands,
            edits,
            lease,
        } = job;
        let job_id = env.get(JOB_ID_ENV).cloned().unwrap_or_default();
        let endpoint = start_process(program, &job_id).await?;
        // A runner that starts next finds the shell by its record, which
        // goes before the job starts; a job the record misses would end with
        // this runner.
        let record = ShellRecord {
            contract: wire::CONTRACT,
            job: job_id.clone(),
            endpoint: endpoint.clone(),
        };
        if let Err(error) = shared.records.write(&record).await {
            tracing::warn!(event = "shell.record_failed", %error, "a job's shell was not recorded; the job ends with this runner");
        }
        let socket = local_socket::connect(&endpoint, &cancellation).await?;
        let (mut reader, mut writer) = tokio::io::split(socket);
        let commands = match commands {
            Some(commands) => Some(Commands {
                context: commands.context,
                roots: commands.roots,
                endpoint: env
                    .get(ENDPOINT_ENV)
                    .cloned()
                    .ok_or_else(|| io::Error::other("a job with commands has no local endpoint"))?,
            }),
            None => None,
        };
        let edits = edits.map(|recorder| {
            let context = recorder.context();
            Edits {
                directory: context.directory.clone(),
                lock: context.lock.clone(),
            }
        });
        for message in [
            ToShell::Hello { contract: wire::CONTRACT },
            ToShell::Start(Box::new(Start {
                job_id: job_id.clone(),
                script,
                cwd,
                env,
                live,
                commands,
                edits,
            })),
        ] {
            writer.write_all(&wire::control(&message)?).await?;
        }
        // The shell says hello, then whether the job started.
        loop {
            match wire::read::<FromShell>(&mut reader).await? {
                Some(Frame::Control(FromShell::Hello { contract, .. })) if contract != wire::CONTRACT => {
                    return Err(io::Error::other(format!(
                        "the job's shell speaks contract {contract}, the runner {}",
                        wire::CONTRACT
                    )));
                }
                Some(Frame::Control(FromShell::Started)) => break,
                Some(Frame::Control(FromShell::StartFailed { reason })) => {
                    return Err(io::Error::other(reason));
                }
                Some(Frame::Control(FromShell::Log { line })) => lines::log_line("shell", &line),
                Some(_) => {}
                None => return Err(io::Error::other("the job's shell ended before its job started")),
            }
        }
        let job = Self::session(reader, writer, lease, [0; 2], job_id, shared);
        // A cancelled job is cancelled in its shell, while it is connected.
        let session = job.session.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = cancellation.cancelled() => session.cancel(),
                () = session.closed.cancelled() => {}
            }
        });
        Ok(job)
    }

    /// Job `job`, whose shell a runner before this one started: none when no
    /// shell is recorded for it, or when `ended` and the job has not ended. A
    /// shell that does not answer, or speaks another contract, has its job
    /// stopped.
    async fn reattach(
        job: &str,
        lease: Option<Box<dyn Send + Sync>>,
        shared: Shared,
        ended: bool,
    ) -> io::Result<Option<Self>> {
        let Some(record) = shared.records.read(job).await? else {
            return Ok(None);
        };
        let reached = tokio::time::timeout(REATTACH_PATIENCE, async {
            let socket = local_socket::connect(&record.endpoint, &CancellationToken::new()).await?;
            let (mut reader, mut writer) = tokio::io::split(socket);
            writer
                .write_all(&wire::control(&ToShell::Hello { contract: wire::CONTRACT })?)
                .await?;
            loop {
                match wire::read::<FromShell>(&mut reader).await? {
                    Some(Frame::Control(FromShell::Hello {
                        contract,
                        started,
                        ended,
                        stdout_kept,
                        stderr_kept,
                        ..
                    })) => return Ok::<_, io::Error>((reader, writer, contract, started, ended, [stdout_kept, stderr_kept])),
                    Some(Frame::Control(FromShell::Log { line })) => lines::log_line("shell", &line),
                    Some(_) => {}
                    None => return Err(io::Error::other("the job's shell ended")),
                }
            }
        })
        .await
        .unwrap_or_else(|_| Err(io::Error::other("the job's shell did not answer")));
        let (reader, mut writer, contract, started, job_ended, kept) = match reached {
            Ok(reached) => reached,
            Err(error) => {
                #[cfg(unix)]
                crate::job::kill_lost(record.job.clone()).await;
                shared.records.remove(&record.job);
                return Err(error);
            }
        };
        // A shell whose runner went before it sent the job ends by itself
        // (`START_PATIENCE`).
        if !started {
            shared.records.remove(&record.job);
            return Err(io::Error::other("the shell's job never started"));
        }
        if contract != wire::CONTRACT {
            shared.records.remove(&record.job);
            // A shell of a contract this runner does not speak still takes
            // these two, whose form no contract changes.
            for message in [ToShell::Cancel, ToShell::ExitKept] {
                writer.write_all(&wire::control(&message)?).await?;
            }
            return Err(io::Error::other(format!(
                "the job's shell speaks contract {contract}, the runner {}",
                wire::CONTRACT
            )));
        }
        // A shell whose job runs is taken over with its job, never alone.
        if ended && !job_ended {
            return Ok(None);
        }
        Ok(Some(Self::session(reader, writer, lease, kept, record.job, shared)))
    }

    /// The job over a connection whose shell holds each stream from its
    /// `kept` bytes on.
    fn session(
        mut reader: tokio::io::ReadHalf<local_socket::Stream>,
        mut writer: tokio::io::WriteHalf<local_socket::Stream>,
        lease: Option<Box<dyn Send + Sync>>,
        kept: [u64; 2],
        job: String,
        shared: Shared,
    ) -> Self {
        let Shared {
            records,
            leaving,
            peers,
        } = shared;
        let (frames, mut queued) = mpsc::unbounded_channel::<Bytes>();
        tokio::spawn(async move {
            while let Some(frame) = queued.recv().await {
                if writer.write_all(&frame).await.is_err() {
                    break;
                }
            }
            let _ = writer.shutdown().await;
        });
        let session = Arc::new(Session {
            frames,
            drains: Mutex::default(),
            next_drain: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            kept: Mutex::new(kept),
            job,
            closed: CancellationToken::new(),
            leaving,
            peers: peers.clone(),
        });
        peers.join(&session);
        let (output_sender, output) = mpsc::channel(16);
        let (outliving_sender, outliving) = watch::channel(Vec::new());
        let (exit_sender, exit) = watch::channel(None);
        let credit = Arc::new(Semaphore::new(INPUT_WINDOW));
        let input_closed = CancellationToken::new();
        let (input, mut inputs) = mpsc::channel::<ProcessInput>(1);
        // The job's input, paced by what it takes.
        {
            let session = session.clone();
            let credit = credit.clone();
            let closed = input_closed.clone();
            tokio::spawn(async move {
                loop {
                    let item = tokio::select! {
                        _ = closed.cancelled() => break,
                        item = inputs.recv() => item,
                    };
                    match item {
                        Some(ProcessInput::Bytes(bytes)) => {
                            let permit = tokio::select! {
                                _ = closed.cancelled() => break,
                                permit = credit.acquire() => permit,
                            };
                            let Ok(permit) = permit else { break };
                            permit.forget();
                            let _ = session.frames.send(wire::input(&bytes));
                        }
                        Some(ProcessInput::End) => {
                            session.send(&ToShell::InputEnd);
                            break;
                        }
                        // A runner that leaves its jobs drops their input,
                        // which the next one carries on.
                        None => {
                            if !session.leaving.load(Ordering::SeqCst) {
                                session.send(&ToShell::InputEnd);
                            }
                            break;
                        }
                    }
                }
            });
        }
        // The shell's messages, until its connection ends.
        {
            let session = session.clone();
            tokio::spawn(async move {
                let mut output_sender = Some(output_sender);
                let mut lease = lease;
                let mut expected = kept;
                let read = async {
                    loop {
                        match wire::read::<FromShell>(&mut reader).await? {
                            Some(Frame::Output { stream, offset, bytes }) => {
                                let index = wire::index(stream);
                                let end = offset + bytes.len() as u64;
                                if end <= expected[index] {
                                    continue;
                                }
                                let bytes = bytes.slice(expected[index].saturating_sub(offset) as usize..);
                                expected[index] = end;
                                if let Some(sender) = &output_sender {
                                    let _ = sender.send(OutputChunk { stream, bytes }).await;
                                }
                            }
                            Some(Frame::Control(message)) => match message {
                                FromShell::Drained { id } => {
                                    let answer = session.drains.lock().expect("the drains are intact").remove(&id);
                                    if let Some(answer) = answer {
                                        let _ = answer.send(());
                                    }
                                }
                                FromShell::Outliving { tasks } => {
                                    outliving_sender.send_replace(tasks);
                                }
                                FromShell::OutputEnd => output_sender = None,
                                FromShell::Exit { code, signal, error } => {
                                    session.ended.store(true, Ordering::SeqCst);
                                    exit_sender.send_replace(Some(ProcessExit { code, signal, error }));
                                }
                                FromShell::Released => lease = None,
                                FromShell::Log { line } => lines::log_line("shell", &line),
                                FromShell::InputTaken => credit.add_permits(1),
                                FromShell::InputClosed => input_closed.cancel(),
                                FromShell::SignalTask { request, task, signal } => {
                                    session.peers.ask(&session, request, task, signal);
                                }
                                FromShell::TaskSignalled { request, found } => {
                                    session.peers.answer(request, found, false);
                                }
                                FromShell::Hello { .. }
                                | FromShell::Started
                                | FromShell::StartFailed { .. }
                                | FromShell::Unknown => {}
                            },
                            Some(Frame::Input(_)) => {}
                            None => return Ok::<_, io::Error>(()),
                        }
                    }
                };
                if let Err(error) = read.await {
                    tracing::warn!(event = "shell.connection_failed", %error, "the connection to a job's shell failed");
                }
                session.closed.cancel();
                let leaving = session.leaving.load(Ordering::SeqCst);
                // A shell that ended without its job's exit took the job
                // with it; the programs it started go too, as a stop's do.
                if exit_sender.borrow().is_none() && !leaving {
                    #[cfg(unix)]
                    crate::job::kill_lost(session.job.clone()).await;
                }
                if exit_sender.borrow().is_none() {
                    session.ended.store(true, Ordering::SeqCst);
                    exit_sender.send_replace(Some(ProcessExit {
                        code: None,
                        signal: None,
                        error: Some("the job's shell ended before its job did".into()),
                    }));
                }
                input_closed.cancel();
                session.drains.lock().expect("the drains are intact").clear();
                // A shell that ended goes with its record; one this runner
                // leaves stays for the next.
                if !leaving {
                    records.remove(&session.job);
                }
                drop(output_sender);
                drop(lease);
            });
        }
        Self {
            input,
            output,
            session,
            outliving,
            exit,
        }
    }
}

/// Starts the shell process for job `job_id` and returns the address it
/// listens on. The shell runs in a session of its own, no child of the
/// runner's on Unix and outside the runner's job objects on Windows, so that
/// it outlives the runner.
async fn start_process(program: &Path, job_id: &str) -> io::Result<String> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(["shell", "--job", job_id])
        .env(JOB_ID_ENV, job_id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // The process forks the shell into a session of its own and ends
    // (`server::serve`).
    #[cfg(unix)]
    let mut child = command.spawn()?;
    #[cfg(windows)]
    let mut child = demi_runner_process::detached::start(|flags| command.creation_flags(flags).spawn())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("the shell process has no standard output"))?;
    // On Unix the process the runner started ends once it forked the
    // shell; the shell's own end is the system's to reap.
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    let mut line = String::new();
    let read = tokio::time::timeout(LISTEN_PATIENCE, BufReader::new(stdout).read_line(&mut line))
        .await
        .map_err(|_| io::Error::other("the job's shell did not say where it listens"))??;
    if read == 0 || line.trim().is_empty() {
        return Err(io::Error::other("the job's shell ended before it listened"));
    }
    Ok(line.trim().to_owned())
}

impl ShellJob for RemoteJob {
    fn input(&self) -> &mpsc::Sender<ProcessInput> {
        &self.input
    }

    fn output(&mut self) -> &mut mpsc::Receiver<OutputChunk> {
        &mut self.output
    }

    fn drain_stdout(&self) -> oneshot::Receiver<()> {
        let (answer, answered) = oneshot::channel();
        if self.session.ended.load(Ordering::SeqCst) {
            // A job that has ended holds nothing more: the dropped answer
            // says so at once.
            return answered;
        }
        let id = self.session.next_drain.fetch_add(1, Ordering::SeqCst);
        self.session
            .drains
            .lock()
            .expect("the drains are intact")
            .insert(id, answer);
        self.session.send(&ToShell::Drain { id });
        answered
    }

    fn signal(&self, signal: Signal) -> io::Result<()> {
        crate::job::stops(signal)?;
        if !self.session.ended.load(Ordering::SeqCst) {
            self.session.send(&ToShell::Signal { signal });
        }
        Ok(())
    }

    fn cancel(&self) {
        self.session.cancel();
    }

    fn is_cancelled(&self) -> bool {
        self.session.cancelled.load(Ordering::SeqCst)
    }

    fn outliving(&self) -> watch::Receiver<Vec<String>> {
        self.outliving.clone()
    }

    fn wait(&mut self) -> BoxFuture<'_, ProcessExit> {
        Box::pin(async move {
            match self.exit.wait_for(Option::is_some).await {
                Ok(exit) => exit.clone().expect("the exit is there"),
                Err(_) => ProcessExit {
                    code: None,
                    signal: None,
                    error: Some("the job's shell ended before its job did".into()),
                },
            }
        })
    }

    fn kept(&self, stream: OutputStream, bytes: u64) {
        let kept = {
            let mut kept = self.session.kept.lock().expect("the counts are intact");
            kept[wire::index(stream)] += bytes;
            *kept
        };
        self.session.send(&ToShell::Kept {
            stdout: kept[0],
            stderr: kept[1],
        });
    }
}

impl Drop for RemoteJob {
    /// Dropping a job that has not ended cancels it, as the contract says.
    fn drop(&mut self) {
        if !self.session.ended.load(Ordering::SeqCst) {
            self.session.cancel();
        }
    }
}
