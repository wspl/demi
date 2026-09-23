//! A Host, its jobs, file contents, callbacks, native commands, and service
//! and network streams over a real runner.

use std::{
    cell::Cell,
    collections::BTreeMap,
    path::Path,
    process::{Output, Stdio},
    rc::Rc,
    time::Duration,
};

use bytes::Bytes;
use demi_command_service::protocol::{CommandCaller, CommandContext};
use demi_command_tree::NativeOperation;
use demi_core::{NodeId, StreamKind};
use demi_host_remote::{
    CommandCatalog, CommandSelection, ContextSource, EnvironmentOptions, JobStart, LogPage, Pipe,
    PipeFailure, PipeReader, RemoteHost, RemoteShellEnvironment, ServiceCallError, ServiceRequest,
    ServiceStream,
    testing::{FixtureOptions, NativeFixture, RunnerFixture, TEST_DEVICE, runner_binary},
};
use demi_runner_protocol::wire::{
    self, ArtifactOwner, JOB_VIEW_BYTES, MAX_MESSAGE_BYTES, Outbound, PipeRef, STDIN_CHUNK_BYTES,
};
use demi_shell::{
    ByteRange, Call, CommandSet, CommandState, CommandStatus, ExecRequest, FileContents,
    GroupBuilder, Host, HostError, HostErrorKind, JobCaller, LeafBuilder, ObservationWindow,
    Process, ProcessEnd, ProcessOutput, RpcError, RpcPort, ShellEnvironment, ShellTarget, Signal,
    SpawnEnv, SpawnRequest, StorageOp, StorageReply, TypedRpc, WriteOptions,
    testing::{host_conformance_cases, test_command_context},
};
use futures_util::{Stream, StreamExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

const MIB: usize = 1024 * 1024;

fn caller() -> JobCaller {
    JobCaller {
        node: NodeId::try_from("test-session").unwrap(),
        generation: 0,
    }
}

fn exec(script: &str, window: u64) -> ExecRequest {
    ExecRequest {
        script: script.into(),
        shell: ShellTarget::Default,
        window: ObservationWindow::from_millis(window).unwrap(),
        caller: caller(),
    }
}

/// Shells on `host` that start with `PATH` and `env`, and whose jobs may run
/// `commands`.
fn shell_on(
    host: RemoteHost,
    env: &[(&str, &str)],
    commands: Option<CommandSelection>,
) -> RemoteShellEnvironment {
    let context: ContextSource = Rc::new(|| Box::pin(async { Ok(test_command_context()) }));
    let mut options = EnvironmentOptions::new(host, context);
    options.initial_env = [("PATH", "/usr/bin:/bin")]
        .iter()
        .chain(env)
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    options.commands = commands;
    RemoteShellEnvironment::new(options)
}

/// Runs `script`, waiting up to ten seconds for its end.
async fn run(shell: &RemoteShellEnvironment, script: &str) -> CommandStatus {
    shell
        .exec(exec(script, 10_000), CancellationToken::new())
        .await
        .unwrap()
}

fn exited(status: &CommandStatus) -> i32 {
    match status.state {
        CommandState::Exited { exit_code, .. } => exit_code,
        ref other => panic!(
            "expected an exited command, got {other:?}: {}",
            status.stderr.tail
        ),
    }
}

/// The hint of a running command.
fn hint(status: &CommandStatus) -> Option<String> {
    match &status.state {
        CommandState::Running { hint } => hint.clone(),
        other => panic!("expected a running command, got {other:?}"),
    }
}

/// Polls `probe` every 20 ms until it yields, at most ten seconds.
async fn until<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
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

/// Bytes that differ at every position, so a misplaced range shows.
fn pattern(size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| ((index * 31 + (index >> 8)) % 251) as u8)
        .collect()
}

async fn collect(mut reader: PipeReader) -> Result<Vec<u8>, PipeFailure> {
    let mut read = Vec::new();
    while let Some(chunk) = reader.next().await {
        read.extend_from_slice(&chunk?);
    }
    Ok(read)
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

    /// How the runner reported the end of pipe `id`: whether it was whole,
    /// and why not.
    async fn pipe_done(&mut self, id: &str) -> (bool, Option<String>) {
        let what = format!("pipe_done for {id}");
        let done = self
            .find(
                &what,
                |message| matches!(message, Outbound::PipeDone { pipe_id, .. } if pipe_id == id),
            )
            .await;
        match done {
            Outbound::PipeDone { ok, error, .. } => (ok, error),
            _ => unreachable!("found a pipe_done"),
        }
    }

    /// Everything that arrived so far.
    fn drain(&mut self) -> &[Outbound] {
        while let Ok(message) = self.receiver.try_recv() {
            self.seen.push(message);
        }
        &self.seen
    }
}

fn spawn(command: &str, args: &[&str]) -> SpawnRequest {
    SpawnRequest {
        command: command.into(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: None,
        env: SpawnEnv::Inherit,
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
async fn the_runner_passes_the_host_conformance_cases_over_its_wire() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let root = format!("{}/conformance", fixture.home());
    std::fs::create_dir(&root).unwrap();
    let host: Rc<dyn Host> = Rc::new(fixture.host_at(&root));
    let mut failures = Vec::new();
    for case in host_conformance_cases(host, &root, "/usr/bin:/bin") {
        let name = case.name;
        if let Err(error) = case.run().await {
            failures.push(format!("{name}: {error}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(fixture.host().online());
    fixture.stop().await;
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
            script: "printf ready; sleep 30".into(),
            cwd: fixture.home().into(),
            env: BTreeMap::new(),
            context: test_command_context(),
            caller: None,
            commands: None,
            stdin: None,
            stdout: None,
        })
        .await
        .unwrap();
    let job_output = futures_util::stream::unfold(&job, |job| async move {
        job.next_output().await.map(|chunk| (chunk, job))
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

#[tokio::test(flavor = "local")]
async fn a_job_runs_on_the_runner_with_its_streams_its_files_and_the_device_environment() {
    let env = BTreeMap::from([
        ("DEVICE_FACT".to_owned(), "from the device".to_owned()),
        ("SHARED".to_owned(), "device".to_owned()),
    ]);
    let fixture = RunnerFixture::start(FixtureOptions {
        env,
        ..FixtureOptions::default()
    })
    .await;
    let shell = shell_on(fixture.host(), &[], None);
    let result = run(&shell, "echo hello; echo oops >&2; exit 4").await;
    assert_eq!(exited(&result), 4);
    assert_eq!(result.stdout.delta, "hello\n");
    assert_eq!(result.stderr.delta, "oops\n");
    // Each stream keeps its bytes, not their order between the streams.
    for (stream, text) in [
        (StreamKind::Stdout, "hello\n"),
        (StreamKind::Stderr, "oops\n"),
    ] {
        let merged: String = result
            .output
            .chunks
            .iter()
            .filter(|chunk| chunk.stream == stream)
            .map(|chunk| chunk.text.as_str())
            .collect();
        assert_eq!(merged, text);
    }
    let directory = result
        .output_dir
        .clone()
        .expect("the runner keeps the output");
    assert_eq!(result.stdout.path, Some(format!("{directory}/stdout.txt")));
    assert_eq!(
        std::fs::read_to_string(format!("{directory}/stdout.txt")).unwrap(),
        "hello\n"
    );
    assert!(!Path::new(&format!("{directory}/cwd")).exists());
    assert_eq!(fixture.link().await.running_jobs(), 0);

    // The device's variables are beneath the shell's; no identity rides in
    // them.
    let facts = run(
        &shell,
        "echo \"$DEVICE_FACT|$SHARED|${DEMI_SESSION_ID:-none}|${DEMI_SHELL_ID:-none}|${DEMI_CONVERSATION_ID:-none}|${DEMI_AGENT_NODE_ID:-none}\"; echo \"$PATH\"",
    )
    .await;
    let (facts, path) = facts.stdout.delta.split_once('\n').unwrap();
    assert_eq!(facts, "from the device|device|none|none|none|none");
    assert!(
        path.trim_end().split(':').any(|entry| entry == "/usr/bin"),
        "{path}"
    );
    let overriding = shell_on(fixture.host(), &[("SHARED", "backend")], None);
    assert_eq!(
        run(&overriding, "echo \"$SHARED\"").await.stdout.delta,
        "backend\n"
    );
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn the_working_directory_carries_between_a_shells_jobs_and_nothing_else_does() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let home = fixture.home().to_owned();
    std::fs::create_dir(format!("{home}/sub")).unwrap();
    let shell = shell_on(fixture.host(), &[], None);
    assert_eq!(
        run(&shell, "cd sub && export FOO=1 && pwd")
            .await
            .stdout
            .delta,
        format!("{home}/sub\n")
    );
    assert_eq!(
        run(&shell, "pwd; echo \"${FOO:-unset}\"")
            .await
            .stdout
            .delta,
        format!("{home}/sub\nunset\n")
    );
    assert_eq!(exited(&run(&shell, "cd ..; exit 3").await), 3);
    assert_eq!(run(&shell, "pwd").await.stdout.delta, format!("{home}\n"));
    // A script that does not parse leaves the directory as it was.
    assert_eq!(exited(&run(&shell, "cd sub; do").await), 2);
    assert_eq!(run(&shell, "pwd").await.stdout.delta, format!("{home}\n"));
    // An ephemeral shell starts where it is told and leaves the default one
    // alone.
    let ephemeral = shell
        .exec(
            ExecRequest {
                script: "pwd".into(),
                shell: ShellTarget::Ephemeral {
                    cwd: Some(format!("{home}/sub")),
                },
                window: ObservationWindow::from_millis(10_000).unwrap(),
                caller: caller(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(ephemeral.stdout.delta, format!("{home}/sub\n"));
    assert_eq!(run(&shell, "pwd").await.stdout.delta, format!("{home}\n"));
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn a_job_outliving_its_window_runs_takes_input_and_can_be_aborted() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let shell = shell_on(fixture.host(), &[], None);
    let running = shell
        .exec(
            exec("echo ready; head -n1; sleep 30", 200),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(running.state, CommandState::Running { .. }));
    until("the head of the view", || {
        (shell.status(&running.command_id).unwrap().stdout.tail == "ready\n").then_some(())
    })
    .await;
    let link = fixture.link().await;
    assert_eq!(link.running_jobs(), 1);
    let written = shell
        .write(&running.command_id, Bytes::from_static(b"typed\n"))
        .await
        .unwrap();
    assert!(matches!(written.state, CommandState::Running { .. }));
    let aborted = shell.abort(&running.command_id).await.unwrap();
    assert!(
        matches!(aborted.state, CommandState::Aborted),
        "{:?}",
        aborted.state
    );
    assert_eq!(link.running_jobs(), 0);
    assert!(matches!(
        shell.status(&running.command_id).unwrap().state,
        CommandState::Aborted
    ));
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn output_beyond_the_view_is_its_head_a_gap_note_and_its_true_tail() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let shell = shell_on(fixture.host(), &[], None);
    let total = 100_000;
    let last = total / 10 - 1;
    let result = run(&shell, &format!("seq -f '%09g' 0 {last}")).await;
    assert_eq!(exited(&result), 0);
    let text = &result.stdout.delta;
    assert!(text.starts_with("000000000\n000000001\n"));
    assert!(text.ends_with(&format!("{last:09}\n")));
    let path = format!("{}/stdout.txt", result.output_dir.clone().unwrap());
    assert!(
        text.contains(&format!("bytes not shown; the full stream is at {path}")),
        "{text}"
    );
    assert!(text.len() < 2 * JOB_VIEW_BYTES + 200);
    assert_eq!(result.stdout.bytes, total as u64);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), total as u64);
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn a_lost_connection_ends_the_job_on_both_sides_and_the_next_connection_serves() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let shell = shell_on(fixture.host(), &[], None);
    let running = shell
        .exec(
            exec("sh -c 'echo $$ > sleeper.pid; exec sleep 30'", 200),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(running.state, CommandState::Running { .. }));
    let pid_file = format!("{}/sleeper.pid", fixture.home());
    let pid = until("the sleeper's pid", || {
        std::fs::read_to_string(&pid_file)
            .ok()
            .filter(|pid| pid.ends_with('\n'))
    })
    .await;
    fixture.link().await.disconnect("the connection was lost");
    let lost = settled(&shell, &running).await;
    assert_eq!(exited(&lost), 127);
    assert!(
        lost.stderr.tail.contains("the connection was lost"),
        "{}",
        lost.stderr.tail
    );
    // The runner stopped the job's process and came back: the same shell
    // serves again.
    fixture.link().await;
    let check = format!(
        "for i in $(seq 100); do kill -0 {} 2>/dev/null || exit 0; sleep 0.05; done; exit 1",
        pid.trim()
    );
    assert_eq!(
        exited(&run(&shell, &check).await),
        0,
        "the job's process outlived its connection"
    );
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn file_contents_travel_whole_or_in_ranges_and_never_in_a_message() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let host = fixture.host();
    let home = fixture.home().to_owned();
    let link = fixture.link().await;
    // Far over the message limit, both ways.
    let large = pattern(5 * MAX_MESSAGE_BYTES + 3);
    std::fs::write(format!("{home}/large.bin"), &large).unwrap();
    assert!(
        host.fs()
            .read_file(&format!("{home}/large.bin"))
            .await
            .unwrap()
            == large
    );
    host.fs()
        .write_file(
            &format!("{home}/copy/large.bin"),
            FileContents::Bytes(large.clone().into()),
            WriteOptions {
                create_parents: true,
            },
        )
        .await
        .unwrap();
    assert!(std::fs::read(format!("{home}/copy/large.bin")).unwrap() == large);
    let copied: Vec<_> = std::fs::read_dir(format!("{home}/copy"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(copied, ["large.bin"]);

    // One range at a time; a file that cannot be read fails before any byte.
    let video = pattern(3 * MIB);
    std::fs::write(format!("{home}/video.bin"), &video).unwrap();
    let range = |offset: usize, length: Option<usize>| {
        let (host, path) = (host.clone(), format!("{home}/video.bin"));
        async move {
            let range = ByteRange {
                offset: offset as u64,
                length: length.map(|length| length as u64),
            };
            let mut stream = host.fs().read_stream(&path, range).await.unwrap();
            let mut read = Vec::new();
            while let Some(chunk) = stream.next().await {
                read.extend_from_slice(&chunk.unwrap());
            }
            read
        }
    };
    assert!(range(MIB + 7, Some(4096)).await == video[MIB + 7..MIB + 7 + 4096]);
    assert!(range(video.len() - 10, None).await == video[video.len() - 10..]);
    assert!(range(0, Some(0)).await.is_empty());
    let refusal = |path: String| {
        let host = host.clone();
        async move {
            host.fs()
                .read_stream(&path, ByteRange::default())
                .await
                .err()
                .expect("a refusal")
        }
    };
    assert_eq!(
        refusal(format!("{home}/missing.bin")).await.code(),
        Some("ENOENT")
    );
    assert_eq!(refusal(home.clone()).await.code(), Some("EISDIR"));

    // A write that cannot land leaves the destination as it was, and no
    // temporary copy.
    let absent = host
        .fs()
        .write_file(
            &format!("{home}/absent/file"),
            FileContents::Bytes(pattern(10).into()),
            WriteOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(absent.code(), Some("ENOENT"));
    std::fs::create_dir(format!("{home}/target")).unwrap();
    std::fs::write(format!("{home}/target/kept"), "kept").unwrap();
    let over_directory = host
        .fs()
        .write_file(
            &format!("{home}/target"),
            FileContents::Bytes(pattern(2 * MIB).into()),
            WriteOptions::default(),
        )
        .await;
    assert!(over_directory.is_err());
    assert_eq!(
        std::fs::read_to_string(format!("{home}/target/kept")).unwrap(),
        "kept"
    );
    let leftovers: Vec<_> = std::fs::read_dir(&home)
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with(".demi-write-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    std::fs::write(format!("{home}/replaced"), "old").unwrap();
    host.fs()
        .write_file(
            &format!("{home}/replaced"),
            FileContents::Bytes(Bytes::from_static(b"new")),
            WriteOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(format!("{home}/replaced")).unwrap(),
        "new"
    );

    // A message over the limit fails its own request, in either direction.
    let request = host
        .fs()
        .stat(&format!("{home}/{}", "x".repeat(MAX_MESSAGE_BYTES)))
        .await
        .unwrap_err();
    assert_eq!(request.kind, HostErrorKind::TooLarge);
    // Long names make a listing outgrow the limit with a few thousand
    // entries.
    std::fs::create_dir(format!("{home}/listing")).unwrap();
    let name = "n".repeat(200);
    for index in 0..=MAX_MESSAGE_BYTES / 200 {
        std::fs::write(format!("{home}/listing/{name}{index}"), "").unwrap();
    }
    let reply = host
        .fs()
        .read_dir(&format!("{home}/listing"))
        .await
        .unwrap_err();
    assert_eq!(reply.kind, HostErrorKind::TooLarge);
    assert!(host.fs().exists(&format!("{home}/listing")).await.unwrap());
    assert!(!link.is_closed(), "the connection stays");
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn a_reader_that_leaves_stops_the_runners_read_and_the_host_keeps_serving() {
    let (sender, mut tap) = Tap::new();
    let fixture = RunnerFixture::start(FixtureOptions {
        tap: Some(sender),
        ..FixtureOptions::default()
    })
    .await;
    let host = fixture.host();
    let long = format!("{}/long.bin", fixture.home());
    std::fs::write(&long, pattern(64 * MIB)).unwrap();
    let mut reader = host.read_pipe(&long, ByteRange::default()).await.unwrap();
    let mut received = 0;
    while received <= MIB {
        received += reader.next().await.unwrap().unwrap().len();
    }
    reader.fail("the preview moved on");
    // The runner's upload failed with the pipe, and it says so.
    tap.find("a failed pipe_done", |message| {
        matches!(message, Outbound::PipeDone { ok: false, .. })
    })
    .await;
    // A stream dropped early is a reader that had enough.
    let mut early = host
        .fs()
        .read_stream(&long, ByteRange::default())
        .await
        .unwrap();
    early.next().await.unwrap().unwrap();
    drop(early);
    assert!(host.fs().exists(&long).await.unwrap());
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn a_jobs_pipes_carry_its_stdin_and_stdout_and_a_refused_end_stops_nothing() {
    let (sender, mut tap) = Tap::new();
    let fixture = RunnerFixture::start(FixtureOptions {
        tap: Some(sender),
        ..FixtureOptions::default()
    })
    .await;
    let host = fixture.host();
    let job = |script: &str, stdin: Option<PipeRef>, stdout: Option<PipeRef>| JobStart {
        script: script.into(),
        cwd: fixture.home().into(),
        env: BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]),
        context: test_command_context(),
        caller: None,
        commands: None,
        stdin,
        stdout,
    };
    // In through the job's standard input, out through its standard output:
    // well past the view, so only the pipes carry it whole.
    let payload: Vec<u8> = (0..3 * MIB)
        .map(|index| b'a' + (index * 7 % 26) as u8)
        .collect();
    let input = fixture.pipes().to_device(TEST_DEVICE);
    let output = fixture.pipes().from_device(TEST_DEVICE);
    let mut writer = input.writer().unwrap();
    let reader = output.reader().unwrap();
    let upper = host
        .start_job(job(
            "tr a-z A-Z",
            Some(input.wire_ref()),
            Some(output.wire_ref()),
        ))
        .await
        .unwrap();
    let feed = async {
        writer.write(Bytes::from(payload.clone())).await.unwrap();
        writer.end();
    };
    let ((), uploaded) = tokio::join!(feed, collect(reader));
    let expected: Vec<u8> = payload.iter().map(u8::to_ascii_uppercase).collect();
    assert!(uploaded.unwrap() == expected, "the upper-cased stream");
    let end = upper.end().await;
    assert_eq!(end.status, ProcessEnd::Exited(0));
    assert_eq!(
        end.output.map(|output| output.stdout_bytes),
        Some(payload.len() as u64)
    );
    assert_eq!(tap.pipe_done(input.id()).await, (true, None));
    assert_eq!(tap.pipe_done(output.id()).await, (true, None));
    let viewed: usize = tap
        .drain()
        .iter()
        .filter_map(|message| match message {
            Outbound::JobOutput { job_id, bytes, .. } if job_id == upper.id() => {
                Some(bytes.0.len())
            }
            _ => None,
        })
        .sum();
    assert!(viewed <= JOB_VIEW_BYTES, "the view stays bounded: {viewed}");

    // A refused standard output is reported, and the job runs to its end
    // rather than wait for a reader that is not there.
    let gone = PipeRef {
        id: "gone".into(),
        url: "/api/pipes/gone".into(),
    };
    let head = host
        .start_job(job(
            "head -c 2000000 /dev/zero; echo done >&2",
            None,
            Some(gone),
        ))
        .await
        .unwrap();
    let end = head.end().await;
    assert_eq!(end.status, ProcessEnd::Exited(0));
    assert_eq!(
        end.output.map(|output| output.stdout_bytes),
        Some(2_000_000)
    );
    let (ok, error) = tap.pipe_done("gone").await;
    assert!(
        !ok && error.as_deref().is_some_and(|error| error.contains("404")),
        "{error:?}"
    );

    // A refused standard input closes it: a reader of it ends.
    let missing = PipeRef {
        id: "missing".into(),
        url: "/api/pipes/missing".into(),
    };
    let count = host
        .start_job(job("wc -c", Some(missing), None))
        .await
        .unwrap();
    let end = count.end().await;
    let tail = end
        .output
        .map(|output| output.stdout_tail.0)
        .unwrap_or_default();
    assert_eq!(String::from_utf8_lossy(&tail).trim(), "0");
    assert!(!tap.pipe_done("missing").await.0);
    fixture.stop().await;
}

/// The input of `todo add` and `todo note`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TodoArgs {
    /// Todo text
    text: String,
}

/// The input of `probe hold`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HoldArgs {
    /// Milliseconds to wait
    ms: u64,
}

async fn add(call: Call<TodoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let text = call.args.text;
    let items = port
        .update("todos", |current: Option<Vec<String>>| {
            let mut items = current.unwrap_or_default();
            items.push(text.clone());
            Ok(items)
        })
        .await?;
    port.stdout(format!("added {}\n", items.len())).await?;
    Ok(0)
}

async fn list(_: Call<Map<String, Value>>, port: RpcPort) -> Result<u8, RpcError> {
    let StorageReply::Value { value, .. } = port
        .storage(StorageOp::Read {
            key: "todos".into(),
        })
        .await?
    else {
        return Err(RpcError::Failed("a read answers a value".into()));
    };
    let items: Vec<String> = value
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| RpcError::Failed(error.to_string()))?
        .unwrap_or_default();
    port.stdout(format!("{}\n", items.join(","))).await?;
    Ok(0)
}

async fn note(call: Call<TodoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    port.stdout(format!("noted: {}", call.args.text)).await?;
    Ok(0)
}

/// Reads live input up to its first line, and answers the line with the
/// call's conversation.
async fn first_line(call: Call<Map<String, Value>>, port: RpcPort) -> Result<u8, RpcError> {
    let mut line = Vec::new();
    while let Some(chunk) = port.read_live_stdin().await? {
        line.extend_from_slice(&chunk);
        if line.contains(&b'\n') {
            break;
        }
    }
    let line = String::from_utf8_lossy(&line);
    port.stdout(format!(
        "{}|{}",
        line.trim_end(),
        call.invocation.context.conversation
    ))
    .await?;
    Ok(0)
}

#[tokio::test(flavor = "local")]
async fn declared_commands_call_back_with_storage_input_and_cancellation() {
    let stopped = Rc::new(Cell::new(false));
    let hold = {
        let stopped = stopped.clone();
        TypedRpc::new(move |call: Call<HoldArgs>, port: RpcPort| {
            let stopped = stopped.clone();
            async move {
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_millis(call.args.ms)) => Ok(0),
                    () = port.cancelled() => {
                        stopped.set(true);
                        Ok(130)
                    }
                }
            }
        })
    };
    let mut commands = CommandSet::new();
    commands
        .register(
            GroupBuilder::new("todo", "Todos.")
                .leaf(
                    LeafBuilder::rpc("add", "Add a todo.")
                        .input::<TodoArgs>()
                        .positionals(["text"])
                        .bind(TypedRpc::new(add)),
                )
                .leaf(LeafBuilder::rpc("list", "List the todos.").bind(TypedRpc::new(list)))
                .leaf(
                    LeafBuilder::rpc("note", "Take a note from stdin.")
                        .input::<TodoArgs>()
                        .stdin_field("text")
                        .bind(TypedRpc::new(note)),
                ),
        )
        .unwrap();
    commands
        .register(
            GroupBuilder::new("probe", "Probes.")
                .leaf(
                    LeafBuilder::rpc("hold", "Wait.")
                        .input::<HoldArgs>()
                        .positionals(["ms"])
                        .bind(hold),
                )
                .leaf(
                    LeafBuilder::rpc("line", "Answer the first line typed.")
                        .bind(TypedRpc::new(first_line)),
                ),
        )
        .unwrap();
    let native = NativeFixture::load();
    let selection = CommandCatalog::new(Vec::new(), native.resolver())
        .unwrap()
        .select(&commands)
        .unwrap();
    let fixture = RunnerFixture::start(FixtureOptions {
        commands,
        ..FixtureOptions::default()
    })
    .await;
    let shell = shell_on(fixture.host(), &[], Some(selection));
    // Command storage, compared and set per agent node.
    let added = run(&shell, "todo add first && todo add second && todo list").await;
    assert_eq!(exited(&added), 0, "{}", added.stderr.tail);
    assert_eq!(added.stdout.delta, "added 1\nadded 2\nfirst,second\n");
    assert_eq!(
        fixture.policy().storage("test-session").value("todos"),
        Some(json!(["first", "second"]))
    );
    // A body from finite standard input.
    let noted = run(&shell, "printf 'from stdin' | todo note").await;
    assert_eq!(noted.stdout.delta, "noted: from stdin");
    // Live input reaches the handler as it is written, with the job's
    // context.
    let typing = shell
        .exec(exec("probe line", 300), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(typing.state, CommandState::Running { .. }));
    shell
        .write(&typing.command_id, Bytes::from_static(b"typed\n"))
        .await
        .unwrap();
    let typed = settled(&shell, &typing).await;
    assert_eq!(exited(&typed), 0);
    assert_eq!(typed.output.tail, "typed|test-conversation");
    // A call its input refuses fails as a usage error.
    let usage = run(&shell, "todo add").await;
    assert_ne!(exited(&usage), 0);
    assert!(!usage.stderr.tail.is_empty());
    // Stopping the job stops its call.
    let holding = shell
        .exec(exec("probe hold 30000", 300), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(holding.state, CommandState::Running { .. }));
    let aborted = shell.abort(&holding.command_id).await.unwrap();
    assert!(
        matches!(aborted.state, CommandState::Aborted),
        "{:?}",
        aborted.state
    );
    until("the call's stop", || stopped.get().then_some(())).await;
    fixture.stop().await;
}

/// The input of the fixture's `where`, which only its schema declares: the
/// native operation reads it.
#[derive(JsonSchema)]
#[schemars(deny_unknown_fields)]
struct WhereArgs {
    /// A label the operation reports
    #[expect(dead_code, reason = "the declaration needs only the schema")]
    label: Option<String>,
}

fn catalog(native: &NativeFixture) -> CommandCatalog {
    CommandCatalog::new(vec![native.descriptor.clone()], native.resolver()).unwrap()
}

fn native_leaf(native: &NativeFixture, name: &str, operation: &str) -> LeafBuilder {
    let operation = NativeOperation {
        package: native.descriptor.id.clone(),
        operation: operation.into(),
    };
    LeafBuilder::native(
        name,
        format!("The fixture's {}.", operation.operation),
        operation,
    )
}

/// A root `name` over the native fixture's operations.
fn native_root(native: &NativeFixture, name: &str) -> CommandSet {
    let mut root = GroupBuilder::new(name, "The native fixture.")
        .leaf(native_leaf(native, "where", "where").input::<WhereArgs>());
    for operation in ["echo", "spin", "result"] {
        root = root.leaf(native_leaf(native, operation, operation));
    }
    let mut commands = CommandSet::new();
    commands.register(root).unwrap();
    commands
}

/// The runner as the root `demi`'s command client, calling `where`.
async fn client(cwd: &str, env: &[(&str, &str)]) -> Output {
    let mut command = tokio::process::Command::new(runner_binary());
    command
        .arg0("demi")
        .arg("where")
        .current_dir(cwd)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .expect("the client ends")
        .unwrap()
}

#[tokio::test(flavor = "local")]
async fn a_native_command_runs_in_its_service_with_the_jobs_context_on_its_own_runner() {
    let native = NativeFixture::load();
    let selection = catalog(&native)
        .select(&native_root(&native, "demi"))
        .unwrap();
    let a = RunnerFixture::start(FixtureOptions::default()).await;
    let b = RunnerFixture::start(FixtureOptions::default()).await;
    let on_a = shell_on(a.host(), &[], Some(selection.clone()));
    let on_b = shell_on(b.host(), &[], Some(selection));
    let (from_a, from_b) = tokio::join!(
        run(
            &on_a,
            "DEMI_CONVERSATION_ID=forged DEMI_AGENT_NODE_ID=forged PROBE=alpha demi where --label A"
        ),
        run(&on_b, "PROBE=beta demi where --label B"),
    );
    let context = serde_json::to_value(test_command_context()).unwrap();
    let reported = |status: &CommandStatus| -> Value {
        serde_json::from_str(&status.stdout.delta)
            .unwrap_or_else(|_| panic!("{}", status.stderr.tail))
    };
    assert_eq!(
        reported(&from_a),
        json!({"context": context, "label": "A", "cwd": a.home(), "value": "alpha"})
    );
    assert_eq!(
        reported(&from_b),
        json!({"context": context, "label": "B", "cwd": b.home(), "value": "beta"})
    );
    // Binary input and output stream through the service whole.
    let bytes: Vec<u8> = (0..3 * MIB).map(|index| (index % 256) as u8).collect();
    std::fs::write(format!("{}/input", a.home()), &bytes).unwrap();
    let echoed = run(&on_a, "cat input | demi echo > output").await;
    assert_eq!(exited(&echoed), 0, "{}", echoed.stderr.tail);
    assert!(std::fs::read(format!("{}/output", a.home())).unwrap() == bytes);
    // An operation's exit code and both its streams; a failure's words.
    let completed = run(&on_a, "demi result").await;
    assert_eq!(exited(&completed), 17);
    assert_eq!(completed.stdout.delta, "command output");
    assert_eq!(completed.stderr.delta, "command diagnostic");
    let failed = run(&on_a, "RESULT=error demi result").await;
    assert_eq!(exited(&failed), 1);
    assert!(
        failed.stderr.delta.contains("command failed"),
        "{}",
        failed.stderr.delta
    );

    // A job's execution context lives on its own runner, and only while the
    // job runs.
    let capture = on_a
        .exec(
            exec("printf '%s\\n%s\\n' \"$DEMI_RUNNER_ENDPOINT\" \"$DEMI_CONTEXT_ID\" > context; sleep 30", 50),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let context_file = format!("{}/context", a.home());
    let written = until("the job's context", || {
        std::fs::read_to_string(&context_file)
            .ok()
            .filter(|text| text.lines().count() == 2)
    })
    .await;
    let (endpoint, context_id) = written.trim_end().split_once('\n').unwrap();
    let other = run(&on_b, "printf '%s' \"$DEMI_RUNNER_ENDPOINT\"")
        .await
        .stdout
        .delta;
    assert_ne!(endpoint, other);
    let wrong = client(
        a.home(),
        &[
            ("DEMI_RUNNER_ENDPOINT", &other),
            ("DEMI_CONTEXT_ID", context_id),
        ],
    )
    .await;
    assert_eq!(wrong.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&wrong.stderr).contains("not live on this runner"),
        "{wrong:?}"
    );
    on_a.abort(&capture.command_id).await.unwrap();
    let stale = client(
        a.home(),
        &[
            ("DEMI_RUNNER_ENDPOINT", endpoint),
            ("DEMI_CONTEXT_ID", context_id),
        ],
    )
    .await;
    assert_eq!(stale.status.code(), Some(1));
    a.stop().await;
    b.stop().await;
}

#[tokio::test(flavor = "local")]
async fn a_running_command_shows_its_leafs_hint_until_the_leaf_ends() {
    let native = NativeFixture::load();
    let mut commands = CommandSet::new();
    commands
        .register(
            GroupBuilder::new("attend", "Hint probes.")
                .leaf(native_leaf(&native, "native", "first").running_hint("native: do not poll"))
                .leaf(native_leaf(&native, "plain", "first"))
                .leaf(
                    LeafBuilder::rpc("rpc", "Wait for a line on the backend.")
                        .running_hint("rpc: do not poll")
                        .bind(TypedRpc::new(first_line)),
                ),
        )
        .unwrap();
    let selection = catalog(&native).select(&commands).unwrap();
    let (sender, mut tap) = Tap::new();
    let fixture = RunnerFixture::start(FixtureOptions {
        commands,
        tap: Some(sender),
        ..FixtureOptions::default()
    })
    .await;
    let shell = shell_on(fixture.host(), &[], Some(selection));
    let shows = |status: &CommandStatus, expected: Option<&str>| {
        let hint = hint(&shell.status(&status.command_id).unwrap());
        (hint.as_deref() == expected).then_some(())
    };
    for leaf in ["native", "rpc"] {
        let expected = format!("{leaf}: do not poll");
        let started = shell
            .exec(
                exec(&format!("attend {leaf}; sleep 30"), 100),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        until(&expected, || shows(&started, Some(&expected))).await;
        shell
            .write(&started.command_id, Bytes::from_static(b"finish\n"))
            .await
            .unwrap();
        until("the hint's end", || shows(&started, None)).await;
        let stopped = shell.abort(&started.command_id).await.unwrap();
        assert!(
            matches!(stopped.state, CommandState::Aborted),
            "{:?}",
            stopped.state
        );
    }

    // A client that dies ends its hint; the shell goes on.
    let child = shell
        .exec(
            exec(
                "exec 9<&0; sh -c 'echo $$ > child.pid; exec attend native' <&9 & wait; sleep 30",
                100,
            ),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    until("the child's hint", || {
        shows(&child, Some("native: do not poll"))
    })
    .await;
    let pid_file = format!("{}/child.pid", fixture.home());
    let pid = until("the child's pid", || {
        std::fs::read_to_string(&pid_file)
            .ok()
            .filter(|pid| pid.ends_with('\n'))
    })
    .await;
    let kill = fixture
        .host()
        .process()
        .spawn(spawn("/bin/kill", &["-KILL", pid.trim()]))
        .await
        .unwrap();
    assert_eq!(finish(kill).await.1, ProcessEnd::Exited(0));
    until("the child's hint to end", || shows(&child, None)).await;
    let stopped = shell.abort(&child.command_id).await.unwrap();
    assert!(
        matches!(stopped.state, CommandState::Aborted),
        "{:?}",
        stopped.state
    );

    // Help, a usage error, a group, and a leaf without a hint show none.
    let before = tap.drain().len();
    for script in ["attend native --help", "attend native --unknown", "attend"] {
        let status = run(&shell, script).await;
        assert!(
            !matches!(status.state, CommandState::Running { .. }),
            "{script}"
        );
    }
    let plain = shell
        .exec(exec("attend plain", 100), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(hint(&plain), None);
    shell
        .write(&plain.command_id, Bytes::from_static(b"done\n"))
        .await
        .unwrap();
    assert_eq!(exited(&settled(&shell, &plain).await), 0);
    let hinted = tap.drain()[before..]
        .iter()
        .any(|message| matches!(message, Outbound::JobRunningHint { hint: Some(_), .. }));
    assert!(!hinted, "a hint from a leaf without one");
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn a_busy_native_command_blocks_neither_the_runner_nor_its_abort_and_a_second_runner_is_refused()
 {
    let native = NativeFixture::load();
    let selection = catalog(&native)
        .select(&native_root(&native, "demi"))
        .unwrap();
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let shell = shell_on(fixture.host(), &[], Some(selection));
    let spinning = shell
        .exec(exec("demi spin", 100), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(spinning.state, CommandState::Running { .. }));
    assert!(fixture.host().fs().exists(fixture.home()).await.unwrap());
    let second = tokio::time::timeout(Duration::from_secs(10), fixture.command().output())
        .await
        .expect("a second runner ends")
        .unwrap();
    assert_eq!(second.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("already active"),
        "{second:?}"
    );
    let aborted = shell.abort(&spinning.command_id).await.unwrap();
    assert!(
        matches!(aborted.state, CommandState::Aborted),
        "{:?}",
        aborted.state
    );
    assert_eq!(exited(&run(&shell, "demi --help").await), 0);
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn a_running_job_keeps_its_manifest_while_the_next_job_installs_another() {
    let native = NativeFixture::load();
    let catalog = catalog(&native);
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let old = shell_on(
        fixture.host(),
        &[],
        Some(catalog.select(&native_root(&native, "demi")).unwrap()),
    );
    let new = shell_on(
        fixture.host(),
        &[],
        Some(
            catalog
                .select(&native_root(&native, "replacement"))
                .unwrap(),
        ),
    );
    let home = fixture.home().to_owned();
    let started = old
        .exec(
            exec("touch ready; while [ ! -f proceed ]; do sleep 0.01; done; PROBE=old demi where > result", 50),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(started.state, CommandState::Running { .. }));
    until("the job to start", || {
        Path::new(&format!("{home}/ready")).exists().then_some(())
    })
    .await;
    let next = run(&new, "PROBE=new replacement where").await;
    let reported: Value =
        serde_json::from_str(&next.stdout.delta).unwrap_or_else(|_| panic!("{}", next.stderr.tail));
    assert_eq!(reported["value"], "new");
    std::fs::write(format!("{home}/proceed"), "").unwrap();
    let finished = settled(&old, &started).await;
    assert_eq!(exited(&finished), 0, "{}", finished.stderr.tail);
    let result: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{home}/result")).unwrap()).unwrap();
    assert_eq!(result["value"], "old");
    fixture.stop().await;
}

/// The conversation's user, as a live view calls a service.
fn user_context() -> CommandContext {
    CommandContext {
        caller: CommandCaller::User {},
        ..test_command_context()
    }
}

fn service(
    fixture: &RunnerFixture,
    native: &NativeFixture,
    operation: &str,
    args: Option<Value>,
) -> ServiceRequest {
    let args = args.map(|args| match args {
        Value::Object(args) => args,
        other => panic!("arguments are an object, not {other}"),
    });
    ServiceRequest {
        context: user_context(),
        package: native.descriptor.clone(),
        operation: operation.into(),
        json: args.is_some().then_some(true),
        args,
        cwd: fixture.home().into(),
        resolver: native.resolver(),
    }
}

/// Opens a service stream with an input and an output pipe.
async fn open(
    fixture: &RunnerFixture,
    request: ServiceRequest,
) -> Result<(Pipe, Pipe, ServiceStream), HostError> {
    let input = fixture.pipes().to_device(TEST_DEVICE);
    let output = fixture.pipes().from_device(TEST_DEVICE);
    let stream = fixture
        .host()
        .open_service(request, input.wire_ref(), output.wire_ref())
        .await?;
    Ok((input, output, stream))
}

#[tokio::test(flavor = "local")]
async fn a_service_stream_carries_bytes_both_ways_and_ends_with_its_invocation() {
    let native = NativeFixture::load();
    let (sender, mut tap) = Tap::new();
    let fixture = RunnerFixture::start(FixtureOptions {
        tap: Some(sender),
        ..FixtureOptions::default()
    })
    .await;
    // The context and the directory reach the invocation; its environment
    // is empty. Its completion ends the output.
    let (input, output, mut stream) = open(&fixture, service(&fixture, &native, "where", None))
        .await
        .unwrap();
    input.writer().unwrap().end();
    let answer = collect(output.reader().unwrap()).await.unwrap();
    let context = serde_json::to_value(user_context()).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&answer).unwrap(),
        json!({"label": null, "context": context, "cwd": fixture.home(), "value": null})
    );
    assert_eq!(stream.done().await.unwrap().exit_code, 0);
    drop(stream);
    // The stream installed the fixture through an artifact request of its
    // own.
    tap.find("a stream's artifact request", |message| {
        matches!(
            message,
            Outbound::ArtifactResolve {
                owner: ArtifactOwner::Stream(_),
                ..
            }
        )
    })
    .await;

    // The page's bytes come back byte-equal; the input's end completes it.
    let (input, output, mut stream) = open(&fixture, service(&fixture, &native, "echo", None))
        .await
        .unwrap();
    let payload = pattern(MIB);
    let mut writer = input.writer().unwrap();
    let feed = async {
        writer.write(Bytes::from(payload.clone())).await.unwrap();
        writer.end();
    };
    let ((), echoed) = tokio::join!(feed, collect(output.reader().unwrap()));
    assert!(echoed.unwrap() == payload, "the echo");
    output.done().await.unwrap();
    assert_eq!(stream.done().await.unwrap().exit_code, 0);
    drop(stream);

    // A package without the operation refuses the stream before any byte.
    let refused = open(&fixture, service(&fixture, &native, "missing", None))
        .await
        .err()
        .unwrap();
    assert_eq!(refused.code(), Some("unknown_operation"));

    // The page going away cancels the invocation, and the runner reports
    // both its pipes.
    let (input, output, stream) = open(&fixture, service(&fixture, &native, "echo", None))
        .await
        .unwrap();
    let mut writer = input.writer().unwrap();
    writer
        .write(Bytes::from_static(b"before the page left"))
        .await
        .unwrap();
    let reader = output.reader().unwrap();
    fixture.pipes().fail(input.id(), "page closed");
    assert!(collect(reader).await.is_err());
    assert!(!tap.pipe_done(input.id()).await.0);
    assert!(!tap.pipe_done(output.id()).await.0);
    drop(stream);
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn a_one_shot_call_learns_its_exit_and_the_host_log_keeps_the_streams_words() {
    let native = NativeFixture::load();
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let host = fixture.host();
    let call = |operation: &str, args: Option<Value>| {
        host.call_service(
            service(&fixture, &native, operation, args),
            Bytes::new(),
            64 * 1024,
        )
    };
    let answer = call("where", Some(json!({"label": "from the user"})))
        .await
        .unwrap();
    let answer: Value = serde_json::from_slice(&answer).unwrap();
    assert_eq!(answer["label"], "from the user");
    assert_eq!(
        answer["context"]["caller"],
        serde_json::to_value(CommandCaller::User {}).unwrap()
    );
    // `result` writes to both outputs and exits 17: the call fails with what
    // it said on standard error.
    match call("result", None).await {
        Err(ServiceCallError::Exited {
            exit_code,
            stderr,
            stdout,
        }) => {
            assert_eq!((exit_code, stderr.as_str()), (17, "command diagnostic"));
            assert_eq!(stdout, "command output");
        }
        other => panic!("expected a failed call, got {other:?}"),
    }
    // What a service holds for a conversation goes when it is released.
    call("retain", None).await.unwrap();
    host.release_conversation(&user_context().conversation)
        .await
        .unwrap();
    assert!(
        open(&fixture, service(&fixture, &native, "missing", None))
            .await
            .is_err()
    );

    // A read waits for no queued line: ask until the writer has put the
    // stream's end in the files.
    let ended = |page: &LogPage| {
        page.lines
            .iter()
            .any(|line| line.text == "stream:result ended")
    };
    let mut page = host.read_log(Some(0), 1000, None).await.unwrap();
    for _ in 0..500 {
        if ended(&page) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        page = host.read_log(Some(0), 1000, None).await.unwrap();
    }
    let conversation = Some(user_context().conversation);
    let told: Vec<_> = page
        .lines
        .iter()
        .map(|line| {
            (
                line.source.as_str(),
                line.conversation_id.clone(),
                line.text.as_str(),
            )
        })
        .collect();
    for (source, text) in [
        ("stream:result", "command diagnostic"),
        ("runner", "stream:result opened"),
        ("runner", "stream:result ended"),
        (
            "runner",
            "stream:missing refused (unknown_operation): demicodes.runner-test has no operation missing",
        ),
    ] {
        assert!(
            told.contains(&(source, conversation.clone(), text)),
            "{source}: {text} in {told:#?}"
        );
    }
    let started = told.iter().any(|(source, _, text)| {
        *source == "runner"
            && text
                .strip_prefix("service demicodes.runner-test started (pid ")
                .and_then(|rest| rest.strip_suffix(')'))
                .is_some_and(|pid| pid.parse::<u32>().is_ok())
    });
    assert!(started, "{told:#?}");
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn the_log_facet_reads_the_newest_lines_then_what_came_after() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let host = fixture.host();
    // The runner writes its start before it connects; the writer puts it in
    // the files a moment later.
    let mut page = host.read_log(None, 50, Some("runner")).await.unwrap();
    for _ in 0..500 {
        if page.lines.iter().any(|line| line.text == "online") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        page = host.read_log(None, 50, Some("runner")).await.unwrap();
    }
    assert!(page.lines.iter().any(|line| line.text == "online"));
    assert!(page.lines.iter().all(|line| line.source == "runner"));
    let started = |text: &str| {
        text.strip_prefix("runner ")
            .and_then(|rest| rest.strip_suffix(" started"))
            .is_some_and(|version| !version.is_empty() && !version.contains(' '))
    };
    assert!(
        page.lines.iter().any(|line| started(&line.text)),
        "{:#?}",
        page.lines
    );
    assert!(page.next > 0);
    let after = host.read_log(Some(page.next), 50, None).await.unwrap();
    assert_eq!((after.lines.len(), after.next), (0, page.next));
    let one = host.read_log(Some(0), 1, None).await.unwrap();
    assert_eq!(one.lines.len(), 1);
    assert!(one.next < page.next);
    let other = host
        .read_log(Some(0), 50, Some("service:none"))
        .await
        .unwrap();
    assert_eq!((other.lines.len(), other.next), (0, page.next));
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn the_working_tree_facet_lists_changes_and_shows_the_last_commit() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let host = fixture.host();
    let home = fixture.home().to_owned();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(&home)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .expect("git on PATH");
        assert!(status.success(), "git {args:?}");
    };
    let outside = host.git_changes(&home).await.unwrap();
    assert!(
        !outside.repository
            && outside.head.is_none()
            && outside.files.is_empty()
            && !outside.truncated
    );
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(format!("{home}/a.txt"), "1\n2\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "first"]);
    std::fs::write(format!("{home}/a.txt"), "1\n2\n3\n").unwrap();
    std::fs::write(format!("{home}/b.txt"), "new\n").unwrap();
    let changes = host.git_changes(&home).await.unwrap();
    assert!(changes.repository);
    let head = changes.head.clone().unwrap();
    assert!(
        head.len() == 40 && head.chars().all(|digit| digit.is_ascii_hexdigit()),
        "{head}"
    );
    let files: Vec<_> = changes
        .files
        .iter()
        .map(|file| {
            (
                file.path.as_str(),
                file.status.as_str(),
                file.kind,
                file.added,
                file.removed,
            )
        })
        .collect();
    assert_eq!(
        files,
        [
            ("a.txt", " M", wire::ChangeKind::Modified, 1, 0),
            ("b.txt", "??", wire::ChangeKind::Added, 1, 0),
        ]
    );
    assert_eq!(host.git_show(&home, "a.txt").await.unwrap(), "1\n2\n");
    assert_eq!(
        host.git_show(&home, "b.txt").await.unwrap_err().code(),
        Some("ENOENT")
    );
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn a_network_stream_carries_a_device_socket_through_two_pipes() {
    let (sender, mut tap) = Tap::new();
    let fixture = RunnerFixture::start(FixtureOptions {
        tap: Some(sender),
        ..FixtureOptions::default()
    })
    .await;
    let host = fixture.host();
    let pipes = fixture.pipes();
    let listener = || async { TcpListener::bind("127.0.0.1:0").await.unwrap() };

    // An echo peer, which ends its side once the runner ended its own: 1 MiB
    // comes back byte-equal, and the input's end half-closes the socket.
    let echo = listener().await;
    let echo_port = echo.local_addr().unwrap().port();
    let echoing = tokio::spawn(async move {
        let (mut socket, _) = echo.accept().await.unwrap();
        let (mut read, mut write) = socket.split();
        tokio::io::copy(&mut read, &mut write).await.unwrap();
        write.shutdown().await.unwrap();
    });
    let input = pipes.to_device(TEST_DEVICE);
    let output = pipes.from_device(TEST_DEVICE);
    let mut writer = input.writer().unwrap();
    let reader = output.reader().unwrap();
    host.open_net("127.0.0.1", echo_port, input.wire_ref(), output.wire_ref())
        .await
        .unwrap();
    let payload: Vec<u8> = (0..MIB)
        .map(|index| b'a' + (index * 13 % 26) as u8)
        .collect();
    let feed = async {
        writer.write(Bytes::from(payload.clone())).await.unwrap();
        writer.end();
    };
    let ((), echoed) = tokio::join!(feed, collect(reader));
    assert!(echoed.unwrap() == payload, "the echo");
    echoing.await.unwrap();
    assert_eq!(tap.pipe_done(input.id()).await, (true, None));
    assert_eq!(tap.pipe_done(output.id()).await, (true, None));

    // A port nobody listens on refuses, with its code.
    let vacant = listener().await;
    let vacant_port = vacant.local_addr().unwrap().port();
    drop(vacant);
    let refused = host
        .open_net(
            "127.0.0.1",
            vacant_port,
            pipes.to_device(TEST_DEVICE).wire_ref(),
            pipes.from_device(TEST_DEVICE).wire_ref(),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Some("refused"));

    // A peer that resets the connection fails the output rather than ends
    // it.
    let reaper = listener().await;
    let reaper_port = reaper.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (socket, _) = reaper.accept().await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        // Unread bytes and no linger make the close a reset.
        socket.set_zero_linger().unwrap();
    });
    let flow = pipes.to_device(TEST_DEVICE);
    let reaped = pipes.from_device(TEST_DEVICE);
    let mut flow_writer = flow.writer().unwrap();
    let reaped_reader = reaped.reader().unwrap();
    host.open_net("127.0.0.1", reaper_port, flow.wire_ref(), reaped.wire_ref())
        .await
        .unwrap();
    let flowing = tokio::task::spawn_local(async move {
        let chunk = Bytes::from(vec![b'x'; 64 * 1024]);
        while flow_writer.write(chunk.clone()).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let (ok, error) = tap.pipe_done(reaped.id()).await;
    assert!(!ok && error.is_some(), "{error:?}");
    assert!(collect(reaped_reader).await.is_err());
    flowing.abort();

    // The backend failing the output mid-upload ends the stream without the
    // peer's end: the socket closes, and both pipes report.
    let speaker = listener().await;
    let speaker_port = speaker.local_addr().unwrap().port();
    let (hung_up, peer_closed) = oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = speaker.accept().await.unwrap();
        socket
            .write_all(b"the peer speaks and then holds the connection open\n")
            .await
            .unwrap();
        let mut rest = Vec::new();
        // The read ends when the runner closes the socket.
        let _ = socket.read_to_end(&mut rest).await;
        let _ = hung_up.send(());
    });
    let held = pipes.to_device(TEST_DEVICE);
    let spoken = pipes.from_device(TEST_DEVICE);
    let held_writer = held.writer().unwrap();
    let mut spoken_reader = spoken.reader().unwrap();
    host.open_net(
        "127.0.0.1",
        speaker_port,
        held.wire_ref(),
        spoken.wire_ref(),
    )
    .await
    .unwrap();
    let first = spoken_reader.next().await.unwrap().unwrap();
    assert!(first.starts_with(b"the peer speaks"));
    spoken_reader.fail("the visitor's connection ended");
    tokio::time::timeout(Duration::from_secs(10), peer_closed)
        .await
        .expect("the socket closes")
        .unwrap();
    let (ok, error) = tap.pipe_done(spoken.id()).await;
    assert!(!ok && error.is_some(), "{error:?}");
    assert!(!tap.pipe_done(held.id()).await.0);
    drop(held_writer);

    // No stream outlives the connection that opened it.
    let quiet = listener().await;
    let quiet_port = quiet.local_addr().unwrap().port();
    let (accepted, connected) = oneshot::channel();
    let (hung_up, peer_closed) = oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = quiet.accept().await.unwrap();
        let _ = accepted.send(());
        let mut rest = Vec::new();
        let _ = socket.read_to_end(&mut rest).await;
        let _ = hung_up.send(());
    });
    let silent = pipes.to_device(TEST_DEVICE);
    let unheard = pipes.from_device(TEST_DEVICE);
    let silent_writer = silent.writer().unwrap();
    let unheard_reader = unheard.reader().unwrap();
    host.open_net(
        "127.0.0.1",
        quiet_port,
        silent.wire_ref(),
        unheard.wire_ref(),
    )
    .await
    .unwrap();
    connected.await.unwrap();
    fixture.link().await.disconnect("the backend went away");
    tokio::time::timeout(Duration::from_secs(10), peer_closed)
        .await
        .expect("the socket closes with its connection")
        .unwrap();
    drop((silent_writer, unheard_reader));
    fixture.stop().await;
}
