//! The job table (`runner.md` § Command lifetime, § Pipes and output): what
//! a job keeps of its output and which views of it the backend is sent,
//! with jobs the test plays through a job shell of its own.

use bytes::Bytes;
use demi_runner_jobs::{
    job_directories::JobDirectories,
    tasks::{JobConfig, JobTable, TaskCommand, TaskSpec, WorkId},
};
use demi_runner_process::{
    job_shell::{JobShell, JobStart, ShellJob},
    pipes::PipeClient,
    process::{OutputChunk, ProcessExit, ProcessInput},
};
use demi_runner_protocol::wire::{
    self, JOB_KEPT_BYTES, JOB_KEPT_PART_BYTES, JOB_LIVE_BYTES, JOB_LIVE_INTERVAL, JOB_VIEW_BYTES,
    KeptRecord, OutputLengths, OutputStream, Signal, WireBytes,
};
use futures_util::{StreamExt, future::BoxFuture};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

#[derive(serde::Deserialize)]
#[serde(tag = "type")]
enum Reply {
    #[serde(rename = "job_output")]
    Output {
        stream: String,
        offset: u64,
        bytes: WireBytes,
    },
    #[serde(rename = "job_exit")]
    Exit {
        #[serde(rename = "exitCode")]
        exit_code: Option<f64>,
        signal: Option<String>,
        output: Option<OutputLengths>,
    },
}

/// A job the test plays: the script it was started with, and its ends of
/// the job's output, input and exit. A job the table cancels sees its
/// cancellation and ends its output, as a shell's job does.
struct Puppet {
    script: String,
    output: mpsc::Sender<OutputChunk>,
    input: mpsc::Receiver<ProcessInput>,
    exit: oneshot::Sender<ProcessExit>,
    cancellation: CancellationToken,
}

impl Puppet {
    /// Prints `bytes` on `stream` in reads of at most 64 KiB, as a pipe
    /// hands them over.
    async fn print(&self, stream: OutputStream, bytes: &[u8]) {
        for chunk in bytes.chunks(64 * 1024) {
            let chunk = OutputChunk {
                stream,
                bytes: Bytes::copy_from_slice(chunk),
            };
            self.output
                .send(chunk)
                .await
                .expect("the table reads the job's output");
        }
    }

    /// Waits for a line of input, as `read` does.
    async fn line(&mut self) {
        let input = self.input.recv().await;
        assert!(matches!(input, Some(ProcessInput::Bytes(bytes)) if bytes == "\n"));
    }

    /// Ends the job's output, then the job with `code`.
    fn exit(self, code: Option<i32>) {
        drop(self.output);
        let exit = ProcessExit {
            code,
            signal: None,
            error: None,
        };
        // A table that closed no longer waits for the exit.
        let _ = self.exit.send(exit);
    }
}

/// A job shell whose jobs the test plays: each start hands it a [`Puppet`].
struct Puppets(mpsc::UnboundedSender<Puppet>);

impl JobShell for Puppets {
    fn start(&self, job: JobStart) -> BoxFuture<'_, io::Result<Box<dyn ShellJob>>> {
        let (output, outputs) = mpsc::channel(4);
        let (input, inputs) = mpsc::channel(4);
        let (exit, exited) = oneshot::channel();
        let puppet = Puppet {
            script: job.script,
            output,
            input: inputs,
            exit,
            cancellation: job.cancellation.clone(),
        };
        let started = self.0.send(puppet).map_err(io::Error::other);
        Box::pin(async move {
            started?;
            Ok(Box::new(Played {
                input,
                output: outputs,
                exit: Some(exited),
                exited: None,
                cancellation: job.cancellation,
            }) as Box<dyn ShellJob>)
        })
    }

    fn builtin_names(&self) -> BTreeSet<String> {
        BTreeSet::new()
    }
}

/// The table's end of a played job.
struct Played {
    input: mpsc::Sender<ProcessInput>,
    output: mpsc::Receiver<OutputChunk>,
    exit: Option<oneshot::Receiver<ProcessExit>>,
    exited: Option<ProcessExit>,
    cancellation: CancellationToken,
}

impl ShellJob for Played {
    fn input(&self) -> &mpsc::Sender<ProcessInput> {
        &self.input
    }

    fn output(&mut self) -> &mut mpsc::Receiver<OutputChunk> {
        &mut self.output
    }

    /// A played job's output is in its channel already.
    fn drain_stdout(&self) -> tokio::sync::oneshot::Receiver<()> {
        let (answer, answered) = tokio::sync::oneshot::channel();
        let _ = answer.send(());
        answered
    }

    fn signal(&self, _: Signal) -> io::Result<()> {
        self.cancellation.cancel();
        Ok(())
    }

    fn cancel(&self) {
        self.cancellation.cancel();
    }

    fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    /// A played job has no background tasks.
    fn outliving(&self) -> tokio::sync::watch::Receiver<Vec<String>> {
        tokio::sync::watch::Sender::new(Vec::new()).subscribe()
    }

    fn wait(&mut self) -> BoxFuture<'_, ProcessExit> {
        Box::pin(async move {
            if let Some(exit) = self.exit.take() {
                let killed = || {
                    ProcessExit {
                        code: None,
                        signal: Some("SIGKILL".into()),
                        error: None,
                    }
                };
                self.exited = Some(tokio::select! {
                    exit = exit => exit.unwrap_or_else(|_| killed()),
                    _ = self.cancellation.cancelled() => killed(),
                });
            }
            self.exited.clone().expect("the exit is kept")
        })
    }
}

/// A job table whose jobs the test plays and keep their output in `root`'s
/// job directories; the jobs arrive at the returned receiver as they start.
async fn table(
    root: &Path,
    capacity: usize,
) -> (
    JobTable,
    mpsc::Receiver<wire::Frame>,
    Arc<JobDirectories>,
    mpsc::UnboundedReceiver<Puppet>,
) {
    let (output, receiver) = mpsc::channel(capacity);
    let token = tokio::sync::watch::Sender::new(Some("test-token".parse().unwrap())).subscribe();
    let pipes = PipeClient::new(&"http://127.0.0.1:1".parse().unwrap(), token).unwrap();
    let directories = JobDirectories::open(root.join("jobs")).await;
    let (started, jobs) = mpsc::unbounded_channel();
    let table = JobTable::new(JobConfig {
        output,
        directories: directories.clone(),
        pipes,
        shell: Arc::new(Puppets(started)),
        commands: None,
    });
    (table, receiver, directories, jobs)
}

/// Starts job `job` with `script` in `root`.
fn start(table: &mut JobTable, root: &Path, script: &str) {
    table
        .start(TaskSpec {
            id: "job".into(),
            cwd: root.into(),
            env: BTreeMap::new(),
            command: TaskCommand::Shell {
                script: script.into(),
                stdin: None,
                stdout: None,
                commands: None,
            },
        })
        .unwrap();
}

/// The job's kept output as it stands, decoded as the backend decodes what
/// `job_read` streams.
async fn kept(directories: &JobDirectories, job: &str) -> Vec<KeptRecord> {
    let reader = directories.output(job).expect("the job's directory");
    let snapshot = tokio::task::spawn_blocking(move || reader.snapshot())
        .await
        .unwrap()
        .unwrap();
    let mut bytes = Vec::new();
    let mut stream = std::pin::pin!(snapshot.into_stream().unwrap());
    while let Some(chunk) = stream.next().await {
        bytes.extend(chunk.unwrap());
    }
    wire::decode_records(&bytes).unwrap()
}

/// The bytes of `stream` that `records` hold, in order.
fn stream_bytes(records: &[KeptRecord], stream: OutputStream) -> Vec<u8> {
    records
        .iter()
        .filter_map(|record| match record {
            KeptRecord::Output(kind, bytes) if *kind == stream => Some(bytes.0.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect()
}

/// A shell job keeps every read of its output, and sends the backend the
/// first `JOB_VIEW_BYTES` of each stream and, unfollowed, how long a stream
/// grew beyond them with its newest `JOB_VIEW_BYTES`, the last of which
/// leaves before its end; its end gives each stream's length, and its
/// directory lasts until its release
/// (`runner.md` § Pipes and output).
#[tokio::test]
async fn a_shell_job_keeps_every_read_and_sends_only_its_views() {
    let root = tempfile::tempdir().unwrap();
    let (mut table, mut receiver, directories, mut jobs) = table(root.path(), 8).await;
    start(&mut table, root.path(), "the views");
    let job = jobs.recv().await.unwrap();
    assert_eq!(job.script, "the views");
    tokio::spawn(async move {
        job.print(OutputStream::Stdout, format!("{:060000}", 0).as_bytes())
            .await;
        job.print(OutputStream::Stderr, format!("{:040000}", 1).as_bytes())
            .await;
        job.exit(Some(0));
    });
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let printed_stdout = format!("{:060000}", 0).into_bytes();
    let printed_stderr = format!("{:040000}", 1).into_bytes();
    // Each stream's newest bytes beyond its first ones, as last sent.
    let mut newest: [Option<(u64, Vec<u8>)>; 2] = [None, None];
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        match rmp_serde::from_slice::<Reply>(&message.into_bytes()).unwrap() {
            Reply::Output {
                stream,
                offset,
                bytes,
            } => {
                let view = if stream == "stdout" {
                    &mut stdout
                } else {
                    &mut stderr
                };
                if offset < JOB_VIEW_BYTES as u64 {
                    // A stream's first bytes come in order.
                    assert_eq!(offset, view.len() as u64);
                    view.extend(bytes.0);
                } else {
                    // Beyond them, a job nobody follows sends how long the
                    // stream grew with its newest bytes, at most as many as
                    // its first ones.
                    let (index, printed) = if stream == "stdout" {
                        (0, &printed_stdout)
                    } else {
                        (1, &printed_stderr)
                    };
                    let end = offset as usize + bytes.0.len();
                    assert!(bytes.0.len() <= JOB_VIEW_BYTES, "{stream} at {offset}");
                    assert_eq!(
                        bytes.0,
                        printed[offset as usize..end],
                        "{stream} at {offset}"
                    );
                    newest[index] = Some((offset, bytes.0));
                }
            }
            Reply::Exit {
                exit_code,
                output,
                signal,
            } => {
                assert_eq!(exit_code, Some(0.0), "{signal:?}");
                let output = output.unwrap();
                assert_eq!(stdout, printed_stdout[..JOB_VIEW_BYTES]);
                assert_eq!(stderr, printed_stderr[..JOB_VIEW_BYTES]);
                assert_eq!(output.stdout_bytes, 60000);
                assert_eq!(output.stderr_bytes, 40000);
                // Before the end, each stream's newest bytes up to its length.
                let newest = newest.map(Option::unwrap);
                assert_eq!(newest[0].0, 60000 - JOB_VIEW_BYTES as u64);
                assert_eq!(newest[1].0, 40000 - JOB_VIEW_BYTES as u64);
                assert_eq!(newest[0].1.len(), JOB_VIEW_BYTES);
                assert_eq!(newest[1].1.len(), JOB_VIEW_BYTES);
                break;
            }
        }
    }
    let records = kept(&directories, "job").await;
    assert_eq!(stream_bytes(&records, OutputStream::Stdout), printed_stdout);
    assert_eq!(stream_bytes(&records, OutputStream::Stderr), printed_stderr);
    table.close().await;
    assert_eq!(table.len(), 0);
    directories.release("job").await;
    assert!(directories.output("job").is_none());
    assert_eq!(job_directories(&directories).len(), 0, "the directory went");
}

/// The job directories under the job root, beside its edit lock.
fn job_directories(directories: &JobDirectories) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(directories.root())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect()
}

/// A job that prints without end holds at most `JOB_KEPT_BYTES` of records:
/// its first `JOB_KEPT_PART_BYTES` whole, its newest after one record that
/// counts the bytes between. About a second: 24 MiB through the job table.
#[tokio::test]
async fn an_endless_job_keeps_its_first_and_last_output_within_the_bound() {
    let root = tempfile::tempdir().unwrap();
    let (mut table, mut receiver, directories, mut jobs) = table(root.path(), 64).await;
    let total = 24 * 1024 * 1024;
    start(&mut table, root.path(), "endless");
    let job = jobs.recv().await.unwrap();
    tokio::spawn(async move {
        let lines: Vec<u8> = b"0123456789\n"
            .iter()
            .copied()
            .cycle()
            .take(usize::try_from(total).unwrap())
            .collect();
        job.print(OutputStream::Stdout, &lines).await;
        job.print(OutputStream::Stdout, b"END").await;
        job.exit(Some(0));
    });
    let lengths = loop {
        if let Reply::Exit { output, .. } = reply(&mut receiver).await {
            break output.unwrap();
        }
    };
    assert_eq!(lengths.stdout_bytes, total + 3);
    let directory = job_directories(&directories)
        .pop()
        .expect("the job's directory");
    let on_disk: u64 = std::fs::read_dir(directory.join("output"))
        .unwrap()
        .map(|file| file.unwrap().metadata().unwrap().len())
        .sum();
    assert!(on_disk <= JOB_KEPT_BYTES as u64, "{on_disk} bytes on disk");
    let records = kept(&directories, "job").await;
    let gap = records
        .iter()
        .position(|record| matches!(record, KeptRecord::LeftOut(_)))
        .expect("a record between the parts");
    let KeptRecord::LeftOut(left_out) = records[gap] else {
        unreachable!()
    };
    let first = stream_bytes(&records[..gap], OutputStream::Stdout);
    let last = stream_bytes(&records[gap + 1..], OutputStream::Stdout);
    assert!(first.starts_with(b"0123456789\n0123456789\n"));
    assert!(last.ends_with(b"01END"));
    assert_eq!(first.len() as u64 + left_out + last.len() as u64, total + 3);
    let head = records[..gap]
        .iter()
        .map(|record| wire::encode_record(record).unwrap().len())
        .sum::<usize>();
    assert!(
        head <= JOB_KEPT_PART_BYTES && head > JOB_KEPT_PART_BYTES - 64 * 1024,
        "{head}"
    );
    table.close().await;
}

/// The next reply of the job's, within a hang guard.
async fn reply(receiver: &mut mpsc::Receiver<wire::Frame>) -> Reply {
    let message = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("a reply within the hang guard")
        .expect("the job's replies");
    rmp_serde::from_slice(&message.into_bytes()).unwrap()
}

/// The next stdout bytes beyond the view, and where they start; what says
/// only a stream's length is passed over.
async fn beyond(receiver: &mut mpsc::Receiver<wire::Frame>) -> (u64, Vec<u8>) {
    loop {
        match reply(receiver).await {
            Reply::Output {
                stream,
                offset,
                bytes,
            } if stream == "stdout" && !bytes.0.is_empty() => {
                assert!(offset >= JOB_VIEW_BYTES as u64, "beyond the view: {offset}");
                return (offset, bytes.0);
            }
            Reply::Output { .. } => {}
            Reply::Exit { .. } => panic!("the job ended"),
        }
    }
}

/// Waits until the job's kept output holds `bytes` of stdout: the runner
/// has read them.
async fn written(directories: &JobDirectories, bytes: usize) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while stream_bytes(&kept(directories, "job").await, OutputStream::Stdout).len() != bytes {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the job's stdout never held {bytes} bytes"));
}

/// While the backend follows a job, the job sends the newest bytes beyond
/// each stream's first `JOB_VIEW_BYTES`: at once when following starts, then
/// at most one message every `JOB_LIVE_INTERVAL`, each of at most
/// `JOB_LIVE_BYTES`, and its last output before its exit; unfollowed, how
/// long a stream grew with its newest `JOB_VIEW_BYTES` (`runner.md` § Pipes
/// and output). About half a
/// second: the job prints a line every 10 ms for that long, so that its
/// output spans a few intervals.
#[tokio::test]
async fn a_followed_job_sends_its_newest_output_beyond_the_view() {
    let root = tempfile::tempdir().unwrap();
    let (mut table, mut receiver, directories, mut jobs) = table(root.path(), 64).await;
    let job = WorkId::Job("job".into());
    start(&mut table, root.path(), "followed");
    let mut played = jobs.recv().await.unwrap();
    tokio::spawn(async move {
        played
            .print(OutputStream::Stdout, format!("{:060000}", 0).as_bytes())
            .await;
        played.line().await;
        for line in 0..50 {
            played
                .print(OutputStream::Stdout, format!("{line:05}\n").as_bytes())
                .await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        played.line().await;
        played
            .print(OutputStream::Stdout, format!("{:020000}", 1).as_bytes())
            .await;
        played.line().await;
        played.print(OutputStream::Stdout, b"end").await;
        played.exit(Some(0));
    });
    let view = JOB_VIEW_BYTES as u64;
    let live = JOB_LIVE_BYTES as u64;

    // Unfollowed: the first `JOB_VIEW_BYTES`, then how long the stream grew
    // with its newest bytes, which end at its length.
    let mut head = 0;
    loop {
        match reply(&mut receiver).await {
            Reply::Output { offset, bytes, .. } if offset < view => {
                assert_eq!(offset, head);
                head += bytes.0.len() as u64;
            }
            Reply::Output { offset, bytes, .. } => {
                assert_eq!(head, view);
                let end = offset + bytes.0.len() as u64;
                assert!(offset >= view && end <= 60_000, "{offset}");
                assert_eq!(bytes.0.len() as u64, view.min(end - view), "{offset}");
                break;
            }
            Reply::Exit { .. } => panic!("the job ended"),
        }
    }

    // Following starts with the newest bytes, at most `JOB_LIVE_BYTES`.
    written(&directories, 60_000).await;
    table.follow(&job, true);
    assert_eq!(
        beyond(&mut receiver).await,
        (60_000 - live, vec![b'0'; JOB_LIVE_BYTES])
    );

    // Then output comes at most once an interval, and nothing is left out
    // of output that fits.
    let started = Instant::now();
    table.input(&job, "\n".into()).unwrap();
    let mut end = 60_000;
    let mut messages = 0;
    while end < 60_300 {
        let (offset, bytes) = beyond(&mut receiver).await;
        assert_eq!(offset, end);
        end += bytes.len() as u64;
        messages += 1;
    }
    let intervals = started.elapsed().as_millis() / JOB_LIVE_INTERVAL.as_millis();
    assert!(
        messages <= intervals + 2,
        "{messages} messages in {:?}",
        started.elapsed()
    );

    // Unfollowed, none of its bytes go.
    table.follow(&job, false);
    table.input(&job, "\n".into()).unwrap();
    written(&directories, 80_300).await;
    while let Ok(message) = receiver.try_recv() {
        let Reply::Output { bytes, .. } = rmp_serde::from_slice(&message.into_bytes()).unwrap()
        else {
            panic!("the job ended");
        };
        assert!(bytes.0.is_empty(), "no bytes while unfollowed");
    }

    // Following again catches up with the newest bytes, and leaves out what
    // does not fit: the end of the 1 printed 20,000 digits wide.
    table.follow(&job, true);
    let mut newest = vec![b'0'; JOB_LIVE_BYTES - 1];
    newest.push(b'1');
    assert_eq!(beyond(&mut receiver).await, (80_300 - live, newest));

    // The last output leaves before the exit.
    table.input(&job, "\n".into()).unwrap();
    let mut last = Vec::new();
    let output = loop {
        match reply(&mut receiver).await {
            Reply::Output { offset, bytes, .. } if !bytes.0.is_empty() => {
                assert_eq!(offset, 80_300 + last.len() as u64);
                last.extend(bytes.0);
            }
            Reply::Output { .. } => {}
            Reply::Exit { output, .. } => break output.unwrap(),
        }
    };
    assert_eq!(last, b"end");
    assert_eq!(output.stdout_bytes, 80_303);
    table.close().await;
}

#[tokio::test]
async fn shutdown_does_not_wait_for_a_blocked_output_consumer() {
    let root = tempfile::tempdir().unwrap();
    let (mut table, mut receiver, _directories, mut jobs) = table(root.path(), 1).await;
    start(&mut table, root.path(), "endless");
    let job = jobs.recv().await.unwrap();
    // Prints until the job is cancelled, as a shell's loop does.
    tokio::spawn(async move {
        let chunk = [b'0'; 4096];
        loop {
            tokio::select! {
                _ = job.cancellation.cancelled() => break,
                sent = job.output.send(OutputChunk {
                    stream: OutputStream::Stdout,
                    bytes: Bytes::copy_from_slice(&chunk),
                }) => if sent.is_err() {
                    break;
                },
            }
        }
    });
    receiver.recv().await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), table.close())
        .await
        .unwrap();
    assert_eq!(table.len(), 0);
}
