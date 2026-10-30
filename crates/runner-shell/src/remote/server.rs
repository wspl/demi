//! The shell process: it runs one job and serves it to whichever runner
//! connects (`mod.rs`).

use std::{
    collections::VecDeque,
    io::{self, Write as _},
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use demi_command_protocol::EditContext;
use demi_command_sdk::edits::Recorder;
use demi_runner_process::{
    job_shell::{JobCommands, JobShell, JobStart, ShellJob},
    local_socket::{Listener, Stream},
    process::{OutputChunk, ProcessExit, ProcessInput},
};
use futures_util::{StreamExt as _, future::BoxFuture, stream::FuturesUnordered};
use tokio::{
    io::AsyncWriteExt as _,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use super::{
    HELD_BYTES,
    forward::Forward,
    wire::{self, Frame, FromShell, Start, ToShell},
};
use crate::ShellRuntime;

/// How long a shell waits for a runner to start its job: a runner that
/// ended between starting the process and sending the job never will.
const START_PATIENCE: Duration = Duration::from_secs(60);

/// The log lines a shell keeps while no runner reads them; the oldest go
/// first, since a shell's lines matter less than its job's output.
const LOG_LINES: usize = 1000;

/// How long a shell whose job has ended waits for a runner to take its exit:
/// a runner that starts again takes its jobs over within seconds, so one
/// that has not come within this will not, and the shell goes.
const UNCLAIMED: Duration = Duration::from_secs(60 * 60);

/// Runs the shell process for job `job`, of release `release`: listens, says
/// where on standard output, and serves the job until it and what it left
/// running have ended and a runner has its exit.
pub fn serve(job: String, release: String) -> io::Result<u8> {
    // The process the runner started ends at once, and the system takes the
    // one that serves, as containerd's shim leaves containerd: a runner that
    // replaced its program in its process would never reap it. The fork
    // comes first, while the process has one thread.
    #[cfg(unix)]
    {
        // SAFETY: no other thread runs yet, so the child copies a whole
        // process.
        match unsafe { libc::fork() } {
            -1 => return Err(io::Error::last_os_error()),
            0 => {
                rustix::process::setsid().map_err(io::Error::from)?;
            }
            _ => return Ok(0),
        }
    }
    let (lines, lines_received) = mpsc::unbounded_channel();
    demi_command_sdk::log_lines_to("shell", job, move |bytes| {
        let line = String::from_utf8_lossy(bytes).trim_end().to_owned();
        let _ = lines.send(line);
    });
    let runtime = ShellRuntime::build()?;
    let shell = ShellRuntime::new(&runtime);
    let result = runtime.block_on(Server::new(shell, release, lines_received).run());
    runtime.shutdown_background();
    result.map(|()| 0)
}

/// The server's events, for a `kill` of this process's job that names a
/// task another job started (`process_builtins.rs`).
static PEERS: std::sync::OnceLock<mpsc::UnboundedSender<Event>> = std::sync::OnceLock::new();

/// How long a `kill` waits for the runner to find the task it names among
/// the other jobs' shells: a runner that does not answer within it is
/// starting again, and the task is taken to be none.
const PEERS_PATIENCE: Duration = Duration::from_secs(5);

/// Signals background task `task` of another job's shell, through the
/// runner: whether a shell had the task; none in a process that serves no
/// runner.
pub(crate) async fn signal_peer_task(task: i32, signal: i32) -> Option<bool> {
    let events = PEERS.get()?;
    let (answer, answered) = oneshot::channel();
    events.send(Event::AskPeers { task, signal, answer }).ok()?;
    tokio::time::timeout(PEERS_PATIENCE, answered).await.ok()?.ok()
}

/// What reaches the server from its connection's reader and its job's input.
enum Event {
    AskPeers {
        task: i32,
        signal: i32,
        answer: oneshot::Sender<bool>,
    },
    Control(u64, ToShell),
    Input(u64, Bytes),
    Closed(u64),
    InputTaken,
    InputClosed,
}

/// The runner connected now.
struct Connection {
    id: u64,
    frames: mpsc::UnboundedSender<Bytes>,
    _reader: AbortOnDropHandle<()>,
    writer: JoinHandle<()>,
}

/// A chunk of output no runner has kept yet.
struct Held {
    stream: demi_runner_protocol::wire::OutputStream,
    offset: u64,
    bytes: Bytes,
}

struct Server {
    shell: ShellRuntime,
    release: String,
    lines: mpsc::UnboundedReceiver<String>,
    events: mpsc::UnboundedSender<Event>,
    received: mpsc::UnboundedReceiver<Event>,
    connection: Option<Connection>,
    connections: u64,
    started: bool,
    job: Option<Box<dyn ShellJob>>,
    output: Option<mpsc::Receiver<OutputChunk>>,
    outliving: Option<watch::Receiver<Vec<String>>>,
    tasks: Vec<String>,
    input: Option<mpsc::UnboundedSender<ProcessInput>>,
    exiting: Option<JoinHandle<ProcessExit>>,
    exit: Option<ProcessExit>,
    exit_kept: bool,
    output_ended: bool,
    release_signal: Option<oneshot::Receiver<()>>,
    released: bool,
    /// Each stream's bytes produced so far, and the runner's kept bytes.
    produced: [u64; 2],
    kept: [u64; 2],
    held: VecDeque<Held>,
    held_bytes: usize,
    drains: FuturesUnordered<BoxFuture<'static, u64>>,
    waiting_lines: VecDeque<String>,
    /// The `kill`s that wait for the runner's answer, by request.
    asks: std::collections::HashMap<u64, oneshot::Sender<bool>>,
    next_ask: u64,
    done: bool,
}

impl Server {
    fn new(shell: ShellRuntime, release: String, lines: mpsc::UnboundedReceiver<String>) -> Self {
        let (events, received) = mpsc::unbounded_channel();
        Self {
            shell,
            release,
            lines,
            events,
            received,
            connection: None,
            connections: 0,
            started: false,
            job: None,
            output: None,
            outliving: None,
            tasks: Vec::new(),
            input: None,
            exiting: None,
            exit: None,
            exit_kept: false,
            output_ended: false,
            release_signal: None,
            released: false,
            produced: [0; 2],
            kept: [0; 2],
            held: VecDeque::new(),
            held_bytes: 0,
            drains: FuturesUnordered::new(),
            waiting_lines: VecDeque::new(),
            asks: std::collections::HashMap::new(),
            next_ask: 0,
            done: false,
        }
    }

    async fn run(mut self) -> io::Result<()> {
        let _ = PEERS.set(self.events.clone());
        let mut listener = Listener::bind(None).await?;
        {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{}", listener.endpoint())?;
            stdout.flush()?;
        }
        let patience = tokio::time::sleep(START_PATIENCE);
        tokio::pin!(patience);
        let unclaimed = tokio::time::sleep(UNCLAIMED);
        tokio::pin!(unclaimed);
        while !self.done {
            let ended_alone = self.exit.is_some() && self.released && self.connection.is_none();
            tokio::select! {
                biased;
                () = &mut patience, if !self.started => {
                    tracing::warn!(event = "shell.unstarted", "no runner started the shell's job");
                    break;
                }
                () = &mut unclaimed, if ended_alone => break,
                socket = listener.accept() => match socket {
                    Ok(socket) => self.connect(socket),
                    Err(error) if demi_command_sdk::descriptors::exhausted(&error) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Err(error) => return Err(error),
                },
                Some(event) = self.received.recv() => self.event(event).await,
                Some(id) = self.drains.next(), if !self.drains.is_empty() => self.drained(id),
                chunk = recv(&mut self.output), if self.output.is_some() && self.held_bytes < HELD_BYTES => match chunk {
                    Some(chunk) => self.hold(chunk),
                    None => self.output_ended(),
                },
                changed = changed(&mut self.outliving), if self.outliving.is_some() => match changed {
                    Some(tasks) => {
                        self.tasks = tasks;
                        self.send(&FromShell::Outliving { tasks: self.tasks.clone() });
                    }
                    None => self.outliving = None,
                },
                exit = async { self.exiting.as_mut().expect("an exit is awaited").await }, if self.exiting.is_some() => {
                    self.exiting = None;
                    let exit = exit.unwrap_or_else(|_| ProcessExit {
                        code: None,
                        signal: None,
                        error: Some("the shell job's wait panicked".into()),
                    });
                    self.send_exit(&exit);
                    self.exit = Some(exit);
                }
                _ = async { self.release_signal.as_mut().expect("a release is awaited").await }, if self.release_signal.is_some() => {
                    self.release_signal = None;
                    self.released = true;
                    self.send(&FromShell::Released);
                }
                Some(line) = self.lines.recv() => self.log(line),
            }
            if self.started && self.exit.is_some() && self.exit_kept && self.released {
                self.done = true;
            }
            if !(self.exit.is_some() && self.released && self.connection.is_none()) {
                unclaimed.as_mut().reset(tokio::time::Instant::now() + UNCLAIMED);
            }
        }
        // What was sent leaves before the process ends.
        if let Some(connection) = self.connection.take() {
            drop(connection.frames);
            let _ = tokio::time::timeout(Duration::from_secs(5), connection.writer).await;
        }
        Ok(())
    }

    /// Takes a runner's connection over from the one before, and sends it what
    /// the shell holds.
    fn connect(&mut self, socket: Stream) {
        self.connections += 1;
        let id = self.connections;
        let (mut reader, mut writer) = tokio::io::split(socket);
        let (frames, mut queued) = mpsc::unbounded_channel::<Bytes>();
        let writer = tokio::spawn(async move {
            while let Some(frame) = queued.recv().await {
                if writer.write_all(&frame).await.is_err() {
                    break;
                }
            }
            let _ = writer.shutdown().await;
        });
        let events = self.events.clone();
        let reader = AbortOnDropHandle::new(tokio::spawn(async move {
            loop {
                match wire::read::<ToShell>(&mut reader).await {
                    Ok(Some(Frame::Control(message))) => {
                        let _ = events.send(Event::Control(id, message));
                    }
                    Ok(Some(Frame::Input(bytes))) => {
                        let _ = events.send(Event::Input(id, bytes));
                    }
                    Ok(Some(Frame::Output { .. })) => {}
                    Ok(None) | Err(_) => break,
                }
            }
            let _ = events.send(Event::Closed(id));
        }));
        self.connection = Some(Connection {
            id,
            frames,
            _reader: reader,
            writer,
        });
        self.send(&FromShell::Hello {
            contract: wire::CONTRACT,
            release: self.release.clone(),
            started: self.started,
            ended: self.exit.is_some(),
            stdout_kept: self.kept[0],
            stderr_kept: self.kept[1],
        });
        if !self.started {
            return;
        }
        let frames: Vec<Bytes> = self
            .held
            .iter()
            .map(|held| wire::output(held.stream, held.offset, &held.bytes))
            .collect();
        for frame in frames {
            self.frame(frame);
        }
        if !self.tasks.is_empty() {
            self.send(&FromShell::Outliving { tasks: self.tasks.clone() });
        }
        if self.output_ended {
            self.send(&FromShell::OutputEnd);
        }
        if let Some(exit) = self.exit.clone() {
            self.send_exit(&exit);
        }
        if self.released {
            self.send(&FromShell::Released);
        }
        while let Some(line) = self.waiting_lines.pop_front() {
            self.send(&FromShell::Log { line });
        }
    }

    async fn event(&mut self, event: Event) {
        match event {
            Event::Control(id, message) if self.current(id) => self.control(message).await,
            Event::Input(id, bytes) if self.current(id) => {
                if let Some(input) = &self.input {
                    let _ = input.send(ProcessInput::Bytes(bytes));
                }
            }
            Event::Closed(id) if self.current(id) => {
                self.connection = None;
                // A runner that went before it started the job never will.
                if !self.started {
                    self.done = true;
                }
            }
            Event::AskPeers { task, signal, answer } => {
                if self.connection.is_none() {
                    let _ = answer.send(false);
                    return;
                }
                self.next_ask += 1;
                self.asks.insert(self.next_ask, answer);
                self.send(&FromShell::SignalTask {
                    request: self.next_ask,
                    task,
                    signal,
                });
            }
            Event::InputTaken => self.send(&FromShell::InputTaken),
            Event::InputClosed => self.send(&FromShell::InputClosed),
            // A message of a connection a newer one replaced.
            _ => {}
        }
    }

    fn current(&self, id: u64) -> bool {
        self.connection.as_ref().is_some_and(|connection| connection.id == id)
    }

    async fn control(&mut self, message: ToShell) {
        match message {
            ToShell::Hello { contract } if contract != wire::CONTRACT => {
                tracing::info!(event = "shell.contract_differed", runner = contract, shell = wire::CONTRACT, "a runner of another contract connected");
            }
            ToShell::Hello { .. } | ToShell::Unknown => {}
            ToShell::Start(start) => {
                if self.started {
                    return;
                }
                self.started = true;
                match self.start(*start).await {
                    Ok(()) => self.send(&FromShell::Started),
                    Err(error) => {
                        self.send(&FromShell::StartFailed {
                            reason: error.to_string(),
                        });
                        self.done = true;
                    }
                }
            }
            ToShell::InputEnd => {
                if let Some(input) = self.input.take() {
                    let _ = input.send(ProcessInput::End);
                }
            }
            ToShell::Drain { id } => match &self.job {
                Some(job) => {
                    let answer = job.drain_stdout();
                    self.drains.push(Box::pin(async move {
                        let _ = answer.await;
                        id
                    }));
                }
                // A job whose output has ended holds nothing more.
                None => self.send(&FromShell::Drained { id }),
            },
            ToShell::Signal { signal } => {
                if let Some(job) = &self.job
                    && let Err(error) = job.signal(signal)
                {
                    tracing::warn!(event = "shell.signal_refused", %error, "a signal the job refused");
                }
            }
            ToShell::Cancel => {
                if let Some(job) = &self.job {
                    job.cancel();
                }
            }
            ToShell::Kept { stdout, stderr } => {
                self.kept = [self.kept[0].max(stdout), self.kept[1].max(stderr)];
                let kept = self.kept;
                self.held
                    .retain(|held| held.offset + held.bytes.len() as u64 > kept[wire::index(held.stream)]);
                self.held_bytes = self.held.iter().map(|held| held.bytes.len()).sum();
            }
            ToShell::ExitKept => self.exit_kept = true,
            ToShell::End => {
                tracing::info!(event = "shell.ended", "the runner stopped, and the shell ends with it");
                std::process::exit(0);
            }
            ToShell::SignalTask { request, task, signal } => {
                let found = match crate::work::Work::signal_task(task, signal) {
                    Some(Ok(())) => true,
                    Some(Err(error)) => {
                        tracing::warn!(event = "shell.task_signal_failed", %error, "another job's signal to a task failed");
                        true
                    }
                    None => false,
                };
                self.send(&FromShell::TaskSignalled { request, found });
            }
            ToShell::TaskSignalled { request, found } => {
                if let Some(answer) = self.asks.remove(&request) {
                    let _ = answer.send(found);
                }
            }
        }
    }

    /// Starts the job in this process's shell.
    async fn start(&mut self, start: Start) -> io::Result<()> {
        let Start {
            job_id: _,
            script,
            cwd,
            env,
            live,
            commands,
            edits,
        } = start;
        let edits = match edits {
            Some(edits) => {
                let context = EditContext {
                    directory: edits.directory,
                    lock: edits.lock,
                };
                let recorder = tokio::task::spawn_blocking(move || Recorder::new(context))
                    .await
                    .map_err(io::Error::other)?;
                match recorder {
                    Ok(recorder) => Some(recorder),
                    Err(error) => {
                        tracing::warn!(event = "edit_record.failed", %error, "a job's edits could not be recorded");
                        None
                    }
                }
            }
            None => None,
        };
        let commands = commands.map(|commands| JobCommands {
            context: commands.context,
            roots: commands.roots,
            handler: Arc::new(Forward {
                endpoint: commands.endpoint,
            }),
        });
        let (released, release_signal) = oneshot::channel();
        let mut job = self
            .shell
            .start(JobStart {
                script,
                cwd,
                env,
                live,
                cancellation: CancellationToken::new(),
                commands,
                edits,
                lease: Some(Box::new(Lease(Some(released)))),
            })
            .await?;
        self.release_signal = Some(release_signal);
        self.output = Some(std::mem::replace(job.output(), mpsc::channel(1).1));
        self.outliving = Some(job.outliving());
        let (input, mut inputs) = mpsc::unbounded_channel();
        let into_job = job.input().clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            while let Some(item) = inputs.recv().await {
                let bytes = matches!(item, ProcessInput::Bytes(_));
                if into_job.send(item).await.is_err() {
                    let _ = events.send(Event::InputClosed);
                    break;
                }
                if bytes {
                    let _ = events.send(Event::InputTaken);
                }
            }
        });
        self.input = Some(input);
        self.job = Some(job);
        Ok(())
    }

    fn hold(&mut self, chunk: OutputChunk) {
        let index = wire::index(chunk.stream);
        let offset = self.produced[index];
        self.produced[index] += chunk.bytes.len() as u64;
        self.held_bytes += chunk.bytes.len();
        self.frame(wire::output(chunk.stream, offset, &chunk.bytes));
        self.held.push_back(Held {
            stream: chunk.stream,
            offset,
            bytes: chunk.bytes,
        });
    }

    /// The job's stdout pipe held what is in its output channel now, which
    /// goes before the answer.
    fn drained(&mut self, id: u64) {
        while let Some(chunk) = self.output.as_mut().and_then(|output| output.try_recv().ok()) {
            self.hold(chunk);
        }
        self.send(&FromShell::Drained { id });
    }

    /// Everything the job ran has finished: its exit follows.
    fn output_ended(&mut self) {
        self.output = None;
        self.output_ended = true;
        self.send(&FromShell::OutputEnd);
        if let Some(mut job) = self.job.take() {
            self.exiting = Some(tokio::spawn(async move { job.wait().await }));
        }
    }

    fn send_exit(&self, exit: &ProcessExit) {
        self.send(&FromShell::Exit {
            code: exit.code,
            signal: exit.signal.clone(),
            error: exit.error.clone(),
        });
    }

    fn log(&mut self, line: String) {
        if self.connection.is_some() {
            self.send(&FromShell::Log { line });
            return;
        }
        if self.waiting_lines.len() == LOG_LINES {
            self.waiting_lines.pop_front();
        }
        self.waiting_lines.push_back(line);
    }

    fn send(&self, message: &FromShell) {
        match wire::control(message) {
            Ok(frame) => self.frame(frame),
            Err(error) => tracing::warn!(event = "shell.message_failed", %error, "a message to the runner could not be encoded"),
        }
    }

    fn frame(&self, frame: Bytes) {
        if let Some(connection) = &self.connection {
            let _ = connection.frames.send(frame);
        }
    }
}

/// The job's lease (`JobStart::lease`), which tells the server when the last
/// of the job's work is gone.
struct Lease(Option<oneshot::Sender<()>>);

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(released) = self.0.take() {
            let _ = released.send(());
        }
    }
}

async fn recv(output: &mut Option<mpsc::Receiver<OutputChunk>>) -> Option<OutputChunk> {
    output.as_mut().expect("the output is read").recv().await
}

async fn changed(outliving: &mut Option<watch::Receiver<Vec<String>>>) -> Option<Vec<String>> {
    let outliving = outliving.as_mut().expect("the tasks are watched");
    outliving.changed().await.ok()?;
    Some(outliving.borrow_and_update().clone())
}
