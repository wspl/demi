//! A Host over a real runner: its commands' input and the link's liveness and
//! reconnects.

use std::{
    collections::BTreeMap,
    rc::Rc,
    time::Duration,
};

use bytes::Bytes;
use demi_backend_remote_host::{
    CommandSelection, ContextSource, EnvironmentOptions, JobStart, RemoteHost, RemoteShellEnvironment,
    testing::{
        FixtureOptions, RunnerFixture,
    },
};
use demi_host_interface::{
    CommandState, CommandStatus, ExecRequest, Host, JobCaller,
    Process, ProcessEnd, ProcessOutput, Seen, ShellEnvironment, ShellError,
    Signal, SpawnEnv, SpawnRequest, Streams,
    testing::{CountingNumbers, TestPages, test_command_context},
};
use demi_runner_protocol::wire::{
    self, Outbound, STDIN_CHUNK_BYTES,
};
use demi_shared_types::{NodeId, StreamKind};
use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const MIB: usize = 1024 * 1024;

pub(crate) fn caller() -> JobCaller {
    JobCaller {
        node: NodeId::try_from("test-session").unwrap(),
        tool_use_id: "call".into(),
        turn: None,
    }
}

/// An exec and how long a call watches it.
pub(crate) struct Exec {
    pub(crate) request: ExecRequest,
    /// In milliseconds.
    pub(crate) window: u64,
}

/// What a job may show a model that reads images, video and PDFs in a tool
/// result.
pub(crate) fn viewable() -> demi_command_protocol::Viewable {
    demi_command_protocol::Viewable {
        model: "test-model".into(),
        media_types: Some(
            ["image/png", "image/jpeg", "video/mp4", "application/pdf"]
                .map(str::to_owned)
                .to_vec(),
        ),
    }
}

/// An exec of `script`, watched for up to `window` milliseconds.
pub(crate) fn exec(script: &str, window: u64) -> Exec {
    Exec {
        request: ExecRequest {
            script: script.into(),
            cwd: None,
            caller: caller(),
            title: "Run the script".into(),
            background: true,
            viewable: viewable(),
        },
        window,
    }
}

/// Starts an exec and watches it for up to its window, as a `shell` call
/// does, or looks at once without one.
pub(crate) trait Watched {
    async fn exec(&self, exec: Exec, cancel: CancellationToken) -> Result<CommandStatus, ShellError>;
}

impl<E: ShellEnvironment> Watched for E {
    async fn exec(&self, exec: Exec, cancel: CancellationToken) -> Result<CommandStatus, ShellError> {
        let command = self.start(exec.request, cancel).await?;
        if exec.window > 0 {
            let window = std::time::Duration::from_millis(exec.window);
            // A command that outlasts the window is looked at as it runs.
            if let Ok(ended) = tokio::time::timeout(window, self.ended(&command)).await {
                ended?;
            }
        }
        self.status(&command)
    }
}

/// An environment on `host` whose jobs start with `PATH` and `env`, and may
/// run `commands`.
pub(crate) fn shell_on(
    host: RemoteHost,
    env: &[(&str, &str)],
    commands: Option<CommandSelection>,
) -> RemoteShellEnvironment {
    let context: ContextSource = Rc::new(|| Box::pin(async { Ok(test_command_context()) }));
    let mut options = EnvironmentOptions::new(
        host,
        context,
        TestPages::new(false),
        Rc::new(CountingNumbers::default()),
    );
    options.initial_env = [("PATH", "/usr/bin:/bin")]
        .iter()
        .chain(env)
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    options.commands = commands;
    RemoteShellEnvironment::new(options)
}

/// Runs `script`, waiting up to ten seconds for its end.
pub(crate) async fn run(shell: &RemoteShellEnvironment, script: &str) -> CommandStatus {
    shell
        .exec(exec(script, 10_000), CancellationToken::new())
        .await
        .unwrap()
}

pub(crate) fn exited(status: &CommandStatus) -> i32 {
    match status.state {
        CommandState::Exited { exit_code, .. } => exit_code,
        ref other => panic!(
            "expected an exited command, got {other:?}: {}",
            status.stderr.tail
        ),
    }
}

/// Polls `probe` every 20 ms until it yields, at most ten seconds.
pub(crate) async fn until<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    for _ in 0..500 {
        if let Some(found) = probe() {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("waited in vain for {what}")
}

/// The command's status once it stopped running.
async fn settled(shell: &RemoteShellEnvironment, status: &CommandStatus) -> CommandStatus {
    until("the command's end", || {
        let status = shell.status(&status.command_id).unwrap();
        (!matches!(status.state, CommandState::Running { .. })).then_some(status)
    })
    .await
}

/// What the runner sent, as its connection read it.
struct Tap {
    receiver: mpsc::Receiver<Outbound>,
    seen: Vec<Outbound>,
}

impl Tap {
    fn new() -> (mpsc::Sender<Outbound>, Self) {
        let (sender, receiver) = mpsc::channel(1 << 16);
        let tap = Self {
            receiver,
            seen: Vec::new(),
        };
        (sender, tap)
    }

    /// The first message `matches` takes, among those seen and those to
    /// come.
    async fn find(&mut self, what: &str, mut matches: impl FnMut(&Outbound) -> bool) -> Outbound {
        if let Some(found) = self.seen.iter().find(|message| matches(message)) {
            return found.clone();
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let message = tokio::time::timeout_at(deadline, self.receiver.recv())
                .await
                .unwrap_or_else(|_| panic!("the runner never sent {what}"))
                .expect("the tap outlives the connection");
            let found = matches(&message);
            self.seen.push(message);
            if found {
                return self.seen.last().expect("just pushed").clone();
            }
        }
    }

}

fn spawn(command: &str, args: &[&str]) -> SpawnRequest {
    SpawnRequest {
        command: command.into(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: None,
        env: SpawnEnv::Inherit,
        descriptors: Vec::new(),
        retained: false,
    }
}

/// Reads `output` until its standard output ends with `text`.
async fn until_stdout(output: impl Stream<Item = ProcessOutput>, text: &str) {
    tokio::pin!(output);
    let mut stdout = Vec::new();
    while !stdout.ends_with(text.as_bytes()) {
        let chunk = output.next().await.expect("output until the text");
        if chunk.stream == StreamKind::Stdout {
            stdout.extend_from_slice(&chunk.bytes);
        }
    }
}

/// A process's standard output and its end.
async fn finish(process: Process) -> (Vec<u8>, ProcessEnd) {
    let Process {
        mut output, exit, ..
    } = process;
    let mut stdout = Vec::new();
    while let Some(chunk) = output.next().await {
        if chunk.stream == StreamKind::Stdout {
            stdout.extend_from_slice(&chunk.bytes);
        }
    }
    (stdout, exit.await)
}

#[tokio::test(flavor = "local")]
async fn a_process_gets_what_was_sent_at_its_start_and_unread_input_blocks_nothing() {
    let (sender, mut tap) = Tap::new();
    let fixture = RunnerFixture::start(FixtureOptions {
        tap: Some(sender),
        ..FixtureOptions::default()
    })
    .await;
    let host = fixture.host();
    // Input and a kill sent while the runner starts the process reach it; a
    // write over one frame arrives whole and in order.
    let cat = host.process().spawn(spawn("/bin/cat", &[])).await.unwrap();
    let large: Vec<u8> = (0..=STDIN_CHUNK_BYTES)
        .map(|index| (index % 256) as u8)
        .collect();
    cat.control
        .write_stdin(Bytes::from_static(b"early input|"))
        .await
        .unwrap();
    cat.control.write_stdin(large.clone().into()).await.unwrap();
    cat.control.close_stdin().await.unwrap();
    let sleeper = host
        .process()
        .spawn(spawn("/bin/sleep", &["10"]))
        .await
        .unwrap();
    sleeper.control.kill(Signal::Kill).await.unwrap();
    let (stdout, end) = finish(cat).await;
    assert!(
        stdout == [b"early input|".as_slice(), &large].concat(),
        "cat's output"
    );
    assert_eq!(end, ProcessEnd::Exited(0));
    assert_eq!(
        finish(sleeper).await.1,
        ProcessEnd::Signalled("SIGKILL".into())
    );

    // Input a job or a process never reads leaves the connection serving,
    // and a kill still ends them.
    let job = host
        .start_job(JobStart {
        id: RemoteHost::job_id(),
            script: "printf ready; sleep 30".into(),
            cwd: fixture.home().into(),
            env: BTreeMap::new(),
            context: test_command_context(),
            viewable: None,
            caller: None,
            commands: None,
            stdin: None,
            stdout: None,
            stop: tokio_util::sync::CancellationToken::new(),
        })
        .await
        .unwrap();
    let job_output = futures_util::stream::unfold(&job, |job| async move {
        job.next_output().await.map(|chunk| {
            let chunk = ProcessOutput {
                stream: chunk.stream,
                bytes: chunk.bytes,
            };
            (chunk, job)
        })
    });
    until_stdout(job_output, "ready").await;
    let mut process = host
        .process()
        .spawn(spawn("/bin/sh", &["-c", "printf ready; sleep 30"]))
        .await
        .unwrap();
    until_stdout(&mut process.output, "ready").await;
    for _ in 0..4 {
        job.write_stdin(Bytes::from(vec![0; STDIN_CHUNK_BYTES]))
            .await
            .unwrap();
        process
            .control
            .write_stdin(Bytes::from(vec![0; STDIN_CHUNK_BYTES]))
            .await
            .unwrap();
    }
    let answered =
        tokio::time::timeout(Duration::from_secs(5), host.fs().exists(fixture.home())).await;
    assert!(
        answered
            .expect("the runner answers while input waits")
            .unwrap()
    );
    job.kill(wire::Signal::Kill).await.unwrap();
    process.control.kill(Signal::Kill).await.unwrap();
    assert_eq!(
        finish(process).await.1,
        ProcessEnd::Signalled("SIGKILL".into())
    );
    job.end().await;
    // A killed job reports the signal beside the shell's exit code.
    let exit = tap
        .find(
            "the job's exit",
            |message| matches!(message, Outbound::JobExit { job_id, .. } if job_id == job.id()),
        )
        .await;
    assert!(
        matches!(&exit, Outbound::JobExit { signal: Some(signal), .. } if signal == "SIGKILL"),
        "{exit:?}"
    );
    fixture.stop().await;
}

/// Starts a job that prints `ready`, waits for it, and never reads its input.
async fn job_that_reads_nothing(host: &RemoteHost, cwd: &str) -> demi_backend_remote_host::RemoteJob {
    let job = host
        .start_job(JobStart {
            id: RemoteHost::job_id(),
            script: "printf ready; sleep 600".into(),
            cwd: cwd.into(),
            env: BTreeMap::new(),
            context: test_command_context(),
            viewable: None,
            caller: None,
            commands: None,
            stdin: None,
            stdout: None,
            stop: CancellationToken::new(),
        })
        .await
        .unwrap();
    let output = futures_util::stream::unfold(&job, |job| async move {
        job.next_output().await.map(|chunk| {
            let chunk = ProcessOutput {
                stream: chunk.stream,
                bytes: chunk.bytes,
            };
            (chunk, job)
        })
    });
    until_stdout(output, "ready").await;
    job
}

/// How long a test waits for what must happen, which only guards against a
/// hang.
const HANG_GUARD: Duration = Duration::from_secs(30);

/// Input that no program reads fills the Host's queue for the runner. When
/// the network also stops passing data while its sockets stay open, the
/// backend's ping waited behind that input, never left, and so never timed
/// out: the device stayed online while its commands hung.
#[tokio::test(flavor = "local")]
async fn a_network_that_passes_no_data_loses_the_device_within_the_ping_deadline() {
    let fixture = RunnerFixture::start(FixtureOptions {
        stallable: true,
        ping: Some(Duration::from_millis(500)),
        ..FixtureOptions::default()
    })
    .await;
    let host = fixture.host();
    let sleeper = host
        .process()
        .spawn(spawn("/bin/sleep", &["600"]))
        .await
        .unwrap();
    fixture.stall_network();
    // Far more than the sockets and the connection's queue hold.
    let flood = sleeper.control.write_stdin(Bytes::from(vec![0; 64 * MIB]));
    tokio::pin!(flood);
    // The write fails once the device is offline; it never succeeds, as the
    // stopped network takes none of it.
    let offline = async {
        tokio::time::timeout(HANG_GUARD, fixture.offline())
            .await
            .expect("the device is offline once its ping went unanswered");
    };
    tokio::select! {
        written = &mut flood => {
            assert!(written.is_err(), "the input was taken by a stopped network: {written:?}");
            tokio::time::timeout(HANG_GUARD, fixture.offline())
                .await
                .expect("the device is offline once its ping went unanswered");
        }
        () = offline => {}
    }
    fixture.stop().await;
}

/// A command whose input nobody reads held its input back until the
/// runner's queue for it was full, and then ended the whole connection
/// with every other command on it.
#[tokio::test(flavor = "local")]
async fn a_command_whose_input_nobody_reads_stalls_only_itself() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let host = fixture.host();
    let job = job_that_reads_nothing(&host, fixture.home()).await;
    let link = fixture.link().await;
    let shell = shell_on(fixture.host(), &[], None);
    let writing = job.write_stdin(Bytes::from(vec![0; 48 * MIB]));
    tokio::pin!(writing);
    let others = async {
        let status = run(&shell, "echo other").await;
        assert_eq!(exited(&status), 0);
        assert_eq!(status.stdout.tail, "other\n");
        assert!(host.fs().exists(fixture.home()).await.unwrap());
    };
    tokio::select! {
        written = &mut writing => panic!("the input of a command that reads none was taken: {written:?}"),
        () = others => {}
    }
    // The connection is the one it was, and the command still runs.
    assert!(!link.is_closed());
    assert!(fixture.link().await == link);
    assert!(job.ended().is_none());
    job.stop().await;
    tokio::time::timeout(HANG_GUARD, &mut writing)
        .await
        .expect("the write ends with the command")
        .ok();
    fixture.stop().await;
}

/// A stop, and a small request, queued behind the input a command does not
/// read, and the input then ended the connection.
#[tokio::test(flavor = "local")]
async fn a_stop_and_a_small_request_do_not_wait_behind_a_commands_input() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let host = fixture.host();
    let job = job_that_reads_nothing(&host, fixture.home()).await;
    let writing = job.write_stdin(Bytes::from(vec![0; 48 * MIB]));
    tokio::pin!(writing);
    let stopped = async {
        // Requests go first, while the input is stuck, then the stop.
        assert!(host.fs().exists(fixture.home()).await.unwrap());
        job.stop().await
    };
    // The write ends with the command, so the stop is looked at first: only a
    // write that ends before it was taken by a command that reads none.
    let end = tokio::select! {
        biased;
        end = tokio::time::timeout(HANG_GUARD, stopped) => end.expect("the stop ends the command"),
        written = &mut writing => panic!("the input of a command that reads none was taken: {written:?}"),
    };
    assert!(
        matches!(end.status, ProcessEnd::Signalled(_) | ProcessEnd::Exited(_)),
        "{:?}",
        end.status
    );
    fixture.stop().await;
}

/// A message the runner cannot decode ends the connection: the runner's
/// close frame names its type and the decoding error, and a process lost
/// with the connection says it, while the job, which outlives connections,
/// goes on over the next (`crates/runner-protocol/src/wire/mod.rs`, `crates/runner/src/connection/kept.rs`). About 0.5 s: one job, a login shell.
#[tokio::test(flavor = "local")]
async fn a_message_the_runner_cannot_decode_ends_the_connection_and_the_lost_process_says_which() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let shell = shell_on(fixture.host(), &[], None);
    let running = shell
        .exec(exec("echo ready; sleep 30", 200), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(running.state, CommandState::Running { .. }));
    let process = fixture
        .host()
        .process()
        .spawn(spawn("/bin/sleep", &["30"]))
        .await
        .unwrap();
    // A newer backend's sync request, with a field this runner does not
    // know: the MessagePack map of two fields gains a third, `shell: 1`.
    let mut newer = wire::encode(&wire::Inbound::Sync { id: "x".into() })
        .unwrap()
        .into_bytes();
    assert_eq!(newer[0], 0x82, "a map of two fields");
    newer[0] = 0x83;
    newer.extend(b"\xa5shell\x01");
    fixture.link().await.send_raw(newer).await.unwrap();
    let ProcessEnd::Lost(reason) = process.exit.await else {
        panic!("the process was not lost with the connection");
    };
    assert!(
        reason.starts_with("runner disconnected: the runner cannot decode sync: unknown field `shell`"),
        "{reason}"
    );
    fixture.link().await;
    assert!(matches!(
        shell.status(&running.command_id).unwrap().state,
        CommandState::Running { .. }
    ));
    shell.abort(&running.command_id).await.unwrap();
    fixture.stop().await;
}

/// A connection loss stops nothing: the runner keeps the job, the output it
/// printed while away reaches the command, and the command ends as the job
/// does (`crates/runner/src/connection/kept.rs`). About a second: the runner
/// connects twice, and the job is a login shell.
#[tokio::test(flavor = "local")]
async fn a_lost_connection_keeps_the_job_and_its_output_and_the_command_ends_as_it_does() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let shell = shell_on(fixture.host(), &[], None);
    let script = "printf 'before\\n'; while [ ! -f go ]; do sleep 0.05; done; printf 'during\\n'; touch printed; while [ ! -f finish ]; do sleep 0.05; done; printf 'after\\n'; exit 3";
    let running = shell
        .exec(exec(script, 200), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(running.state, CommandState::Running { .. }));
    let home = std::path::PathBuf::from(fixture.home());
    // The connection ends; the runner connects again as soon as the backend
    // lets go of the old one, so the end is the old link's, not the device's
    // being offline, which a reconnect may already have undone.
    let lost = fixture.link().await;
    lost.disconnect("the connection was lost");
    lost.ended().await;
    // The job prints while its runner is away or on its way back.
    std::fs::write(home.join("go"), "").unwrap();
    until("the job printed while away", || home.join("printed").exists().then_some(())).await;
    // The runner is back, and the command goes on there.
    fixture.link().await;
    std::fs::write(home.join("finish"), "").unwrap();
    let ended = settled(&shell, &running).await;
    assert_eq!(exited(&ended), 3);
    let whole = ended.whole.expect("the whole output");
    let stdout = whole
        .output
        .text(Streams::Only(StreamKind::Stdout), None, Seen::default());
    assert_eq!(
        String::from_utf8_lossy(stdout.bytes()),
        "before\nduring\nafter\n"
    );
    fixture.stop().await;
}
