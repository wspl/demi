use demi_runner::connection::wire::{
    self, JOB_KEPT_BYTES, JOB_KEPT_PART_BYTES, JOB_LIVE_BYTES, JOB_LIVE_INTERVAL, JOB_VIEW_BYTES,
    KeptRecord, OutputLengths, OutputStream, Signal, WireBytes,
};
use demi_runner::{
    job_directories::JobDirectories,
    pipes::PipeClient,
    tasks::{JobConfig, JobTable, TaskCommand, TaskSpec, WorkId},
};
use futures_util::StreamExt;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::Instant};

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
        cwd: Option<String>,
        output: Option<OutputLengths>,
    },
}

/// A job table whose jobs keep their output in `root`'s job directories.
async fn table(
    root: &Path,
    capacity: usize,
) -> (
    JobTable,
    mpsc::Receiver<demi_runner::connection::wire::Frame>,
    Arc<JobDirectories>,
) {
    let (output, receiver) = mpsc::channel(capacity);
    let token = tokio::sync::watch::Sender::new(Some("test-token".parse().unwrap())).subscribe();
    let pipes = PipeClient::new(&"http://127.0.0.1:1".parse().unwrap(), token).unwrap();
    let directories = JobDirectories::open(root.join("jobs")).await;
    (
        JobTable::new(JobConfig {
            output,
            directories: directories.clone(),
            pipes,
            shell: demi_runner::shell::ShellRuntime::current(),
            commands: None,
        }),
        receiver,
        directories,
    )
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

/// Waits for a shell job's complete readiness marker across output frames.
async fn wait_for_job_ready(job: &mut demi_runner::shell::job::Job) {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut ready = Vec::new();
        while ready.len() < 5 {
            let chunk = job.output.recv().await.expect("job exited before ready");
            ready.extend(chunk.bytes);
        }
        assert_eq!(ready, b"ready");
    })
    .await
    .unwrap();
}

/// A shell job keeps every read of its output, and sends the backend the
/// first `JOB_VIEW_BYTES` of each stream and, unfollowed, only how long a
/// stream grew beyond them; its end gives each stream's length, and its
/// directory lasts until its release (`runner.md` § Pipes and output).
#[tokio::test]
async fn a_shell_job_keeps_every_read_and_sends_only_its_views() {
    let root = tempfile::tempdir().unwrap();
    let (mut table, mut receiver, directories) = table(root.path(), 8).await;
    table
        .start(TaskSpec {
            id: "job".into(),
            cwd: root.path().into(),
            env: crate::home(root.path()),
            command: TaskCommand::Shell {
                script: "printf '%060000d' 0; printf '%040000d' 1 >&2; mkdir child; cd child"
                    .into(),
                stdin: None,
                stdout: None,
                commands: None,
            },
        })
        .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
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
                    // Beyond them, a job nobody follows sends only how long
                    // the stream grew.
                    assert!(bytes.0.is_empty(), "{stream} at {offset}");
                    let total = if stream == "stdout" { 60000 } else { 40000 };
                    assert!(offset <= total, "{stream} at {offset}");
                }
            }
            Reply::Exit {
                exit_code,
                cwd,
                output,
                signal,
            } => {
                assert_eq!(exit_code, Some(0.0), "{signal:?}");
                assert_eq!(
                    std::fs::canonicalize(cwd.unwrap()).unwrap(),
                    root.path().join("child").canonicalize().unwrap()
                );
                let output = output.unwrap();
                assert_eq!(stdout.len(), 32768);
                assert_eq!(stderr.len(), 32768);
                assert_eq!(output.stdout_bytes, 60000);
                assert_eq!(output.stderr_bytes, 40000);
                break;
            }
        }
    }
    let records = kept(&directories, "job").await;
    let mut printed = format!("{:060000}", 0).into_bytes();
    assert_eq!(stream_bytes(&records, OutputStream::Stdout), printed);
    printed = format!("{:040000}", 1).into_bytes();
    assert_eq!(stream_bytes(&records, OutputStream::Stderr), printed);
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
/// counts the bytes between. About a second: 24 MiB through the runner.
#[tokio::test]
async fn an_endless_job_keeps_its_first_and_last_output_within_the_bound() {
    let root = tempfile::tempdir().unwrap();
    let (mut table, mut receiver, directories) = table(root.path(), 64).await;
    let total = 24 * 1024 * 1024;
    table
        .start(TaskSpec {
            id: "job".into(),
            cwd: root.path().into(),
            env: crate::home(root.path()),
            command: TaskCommand::Shell {
                script: format!("yes 0123456789 | head -c {total}; printf END"),
                stdin: None,
                stdout: None,
                commands: None,
            },
        })
        .unwrap();
    let lengths = loop {
        if let Reply::Exit { output, .. } = reply(&mut receiver).await {
            break output.unwrap();
        }
    };
    assert_eq!(lengths.stdout_bytes, total + 3);
    let directory = job_directories(&directories).pop().expect("the job's directory");
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
    assert!(head <= JOB_KEPT_PART_BYTES && head > JOB_KEPT_PART_BYTES - 64 * 1024, "{head}");
    table.close().await;
}

/// The next reply of the job's, within a hang guard.
async fn reply(receiver: &mut mpsc::Receiver<demi_runner::connection::wire::Frame>) -> Reply {
    let message = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("a reply within the hang guard")
        .expect("the job's replies");
    rmp_serde::from_slice(&message.into_bytes()).unwrap()
}

/// The next stdout bytes beyond the view, and where they start; what says
/// only a stream's length is passed over.
async fn beyond(receiver: &mut mpsc::Receiver<demi_runner::connection::wire::Frame>) -> (u64, Vec<u8>) {
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
/// `JOB_LIVE_BYTES`, and its last output before its exit; unfollowed, only
/// how long a stream grew (`runner.md` § Pipes and output). About 1.5 s: a
/// login shell, and a loop that prints for about half a second, so that its
/// output spans a few intervals.
#[tokio::test]
async fn a_followed_job_sends_its_newest_output_beyond_the_view() {
    let root = tempfile::tempdir().unwrap();
    let (mut table, mut receiver, directories) = table(root.path(), 64).await;
    let job = WorkId::Job("job".into());
    table
        .start(TaskSpec {
            id: "job".into(),
            cwd: root.path().into(),
            env: crate::home(root.path()),
            command: TaskCommand::Shell {
                script: "printf '%060000d' 0; read a; i=0; while [ $i -lt 50 ]; do printf '%05d\n' $i; i=$((i+1)); sleep 0.01; done; read b; printf '%020000d' 1; read c; printf end".into(),
                stdin: None,
                stdout: None,
                commands: None,
            },
        })
        .unwrap();
    let view = JOB_VIEW_BYTES as u64;
    let live = JOB_LIVE_BYTES as u64;

    // Unfollowed: the first 32 KiB, then only how long the stream grew.
    let mut head = 0;
    loop {
        match reply(&mut receiver).await {
            Reply::Output { offset, bytes, .. } if offset < view => {
                assert_eq!(offset, head);
                head += bytes.0.len() as u64;
            }
            Reply::Output { offset, bytes, .. } => {
                assert!(bytes.0.is_empty());
                assert_eq!(head, view);
                assert!(offset > view && offset <= 60_000, "{offset}");
                break;
            }
            Reply::Exit { .. } => panic!("the job ended"),
        }
    }

    // Following starts with the newest bytes, at most `JOB_LIVE_BYTES`.
    written(&directories, 60_000).await;
    table.follow(&job, true);
    assert_eq!(beyond(&mut receiver).await, (60_000 - live, vec![b'0'; JOB_LIVE_BYTES]));

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
        let Reply::Output { bytes, .. } = rmp_serde::from_slice(&message.into_bytes()).unwrap() else {
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
async fn functions_and_compound_pipelines_drain_large_output_and_here_documents() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("input"), vec![b'x'; 262_144]).unwrap();
    let (mut table, mut receiver, _directories) = table(root.path(), 8).await;
    table
        .start(TaskSpec {
            id: "pipeline".into(),
            cwd: root.path().into(),
            env: crate::home(root.path()),
            command: TaskCommand::Shell {
                script: "producer() { cat input; }; value=$(producer | cat | cat); printf '%s\\n' \"${#value}\"; { producer; } | wc -c; (producer) | wc -c; cat <<EOF | wc -c\n$value\nEOF\ncat <<< \"$value\" | wc -c".into(),
                stdin: None,
                stdout: None,
                commands: None,
            },
        })
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(60), async {
        let mut output = Vec::new();
        loop {
            let message = receiver.recv().await.unwrap();
            match rmp_serde::from_slice::<Reply>(&message.into_bytes()).unwrap() {
                Reply::Output { stream, bytes, .. } => {
                    assert_eq!(stream, "stdout", "{:?}", bytes.0);
                    output.extend(bytes.0);
                }
                Reply::Exit {
                    exit_code, signal, ..
                } => {
                    assert_eq!(exit_code, Some(0.0), "{signal:?}");
                    return output;
                }
            }
        }
    })
    .await;
    table.close().await;
    let output = String::from_utf8(result.expect("pipeline deadlocked")).unwrap();
    assert_eq!(
        output.split_whitespace().collect::<Vec<_>>(),
        ["262144", "262144", "262144", "262145", "262145"]
    );
}

#[tokio::test]
async fn cancellation_terminates_a_blocking_native_builtin() {
    let root = tempfile::tempdir().unwrap();
    let (mut table, mut receiver, _directories) = table(root.path(), 8).await;
    table
        .start(TaskSpec {
            id: "job".into(),
            cwd: root.path().into(),
            env: crate::home(root.path()),
            command: TaskCommand::Shell {
                script: "printf ready; sleep 60".into(),
                stdin: None,
                stdout: None,
                commands: None,
            },
        })
        .unwrap();
    let message = receiver.recv().await.unwrap();
    assert!(matches!(
        rmp_serde::from_slice::<Reply>(&message.into_bytes()).unwrap(),
        Reply::Output { .. }
    ));
    table
        .signal(&WorkId::Job("job".into()), Signal::Kill)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(message) = receiver.recv().await {
            if matches!(
                rmp_serde::from_slice::<Reply>(&message.into_bytes()).unwrap(),
                Reply::Exit { .. }
            ) {
                return;
            }
        }
        panic!("task closed without an exit reply");
    })
    .await
    .unwrap();
    table.close().await;
    assert_eq!(table.len(), 0);
}

/// Each signal runs in a job of its own, and the jobs run together: each is a
/// login shell that reads the machine's profile first.
#[tokio::test]
async fn shell_cancellation_reports_the_requesting_signal() {
    use demi_runner::shell::{job::Job, scope::Scope};
    use tokio_util::sync::CancellationToken;

    let signals = [
        Some(Signal::Terminate),
        Some(Signal::Interrupt),
        Some(Signal::Hangup),
        Some(Signal::Quit),
        Some(Signal::Kill),
        None,
    ]
    .map(|signal| async move {
        let root = tempfile::tempdir().unwrap();
        let scope = Scope::new(CancellationToken::new(), None);
        let mut job = Job::start(
            "printf ready; sleep 60".into(),
            root.path().into(),
            crate::home(root.path()),
            true,
            scope.clone(), &demi_runner::shell::ShellRuntime::current(),
        )
            .await
        .unwrap();
        wait_for_job_ready(&mut job).await;
        assert!(job.signal(Signal::User1).await.is_err());
        assert!(!job.is_cancelled());
        if let Some(signal) = signal {
            job.signal(signal).await.unwrap();
            // Cleanup or repeated requests cannot replace the original cause.
            job.signal(Signal::Kill).await.unwrap();
        } else {
            job.cancel();
            job.signal(Signal::Terminate).await.unwrap();
        }
        let (exit, _) = tokio::time::timeout(Duration::from_secs(3), job.wait())
            .await
            .unwrap();
        let expected = signal.map_or("SIGKILL".to_owned(), |signal| signal.to_string());
        assert_eq!(exit.signal, Some(expected));
        // Bash's own exit code is its interruption's, not the job's.
        assert_eq!(exit.code, None);
        assert!(exit.error.is_none());
        assert_eq!(scope.tasks.len(), 0);
    });
    futures_util::future::join_all(signals).await;
}

#[tokio::test]
async fn shutdown_does_not_wait_for_a_blocked_output_consumer() {
    let root = tempfile::tempdir().unwrap();
    let (mut table, mut receiver, _directories) = table(root.path(), 1).await;
    table
        .start(TaskSpec {
            id: "spawn".into(),
            cwd: root.path().into(),
            env: crate::home(root.path()),
            command: TaskCommand::Shell {
                script: "while :; do printf '%04096d' 0; done".into(),
                stdin: None,
                stdout: None,
                commands: None,
            },
        })
        .unwrap();
    receiver.recv().await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), table.close())
        .await
        .unwrap();
    assert_eq!(table.len(), 0);
}

/// Waits until the job of `scope` blocks or loops: one of its units waits
/// inside an interruptible read, write or sleep, or the job checks for
/// cancellation another hundred times, as a loop does at every step.
#[cfg(feature = "test-fixtures")]
async fn blocked_or_looping(scope: &demi_runner::shell::scope::Scope, script: &str) {
    let activity = scope.activity();
    let checks = activity.checks();
    tokio::time::timeout(Duration::from_secs(60), async {
        while activity.waiting() == 0 && activity.checks() < checks + 100 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{script} neither blocks nor loops"));
}

/// A job that blocks, in any of the ways below, ends with `SIGKILL` and no
/// work left when it is cancelled there. The blocked jobs run at once beside
/// a sibling job, which prints the runner's process as its `$$`, waits in
/// `read` through every cancellation and then finishes as it would alone.
/// About 4 s: each way of blocking needs a job of its own, and each job is a
/// login shell that reads the machine's profile first (about 0.4 s in the
/// Linux container, with nvm), so the jobs start together.
///
/// The jobs run on a shell runtime of their own, as in the runner, which the
/// test shuts down without waiting: a unit that ignores cancellation fails
/// the test when its job does not end, rather than holding the test's
/// runtime, which would wait for it at shutdown.
#[cfg(feature = "test-fixtures")]
#[tokio::test]
async fn jobs_share_the_runner_process_and_cancellation_is_isolated() {
    use demi_runner::{
        process::{OutputStream, ProcessInput},
        shell::{ShellRuntime, job::Job, scope::Scope},
    };
    use tokio_util::sync::CancellationToken;
    // Left undropped when the test fails, since dropping waits for its units.
    let shell = std::mem::ManuallyDrop::new(ShellRuntime::build().unwrap());
    let runtime = &ShellRuntime::new(&shell);
    let root = tempfile::tempdir().unwrap();
    let mut sibling = Job::start(
        "printf '%s' $$; read go; printf done".into(),
        root.path().into(),
        crate::home(root.path()),
        false,
        Scope::new(CancellationToken::new(), None),
        runtime,
    )
    .await
    .unwrap();
    let process = std::process::id().to_string();
    let mut printed = Vec::new();
    while printed.len() < process.len() {
        let chunk = tokio::time::timeout(Duration::from_secs(60), sibling.output.recv())
            .await
            .unwrap()
            .expect("the sibling prints its process");
        assert!(matches!(chunk.stream, OutputStream::Stdout));
        printed.extend(chunk.bytes);
    }
    assert_eq!(String::from_utf8(printed).unwrap(), process);
    let blocked = [
        "while :; do :; done",
        "printf line | sed ':again; b again'",
        "jq -n 'def forever: forever; forever'",
        "cat",
        "tee file",
        "wc -c",
        "head -c 99999",
        "tail -c +1",
        "od -j 99999",
        "(sleep 60) & wait",
        "cat <(sleep 60)",
        "touch file; tail -f -s 60 file",
    ]
    .map(|script| async move {
        let root = tempfile::tempdir().unwrap();
        let scope = Scope::new(CancellationToken::new(), None);
        let mut job = Job::start(
            format!("printf ready; {script}"),
            root.path().into(),
            crate::home(root.path()),
            true,
            scope.clone(),
            runtime,
        )
        .await
        .unwrap();
        wait_for_job_ready(&mut job).await;
        blocked_or_looping(&scope, script).await;
        job.cancel();
        let (exit, _) = tokio::time::timeout(Duration::from_secs(3), job.wait())
            .await
            .expect(script);
        assert_eq!(exit.signal.as_deref(), Some("SIGKILL"), "{script}");
        assert_eq!(scope.tasks.len(), 0, "{script}");
    });
    futures_util::future::join_all(blocked).await;
    sibling
        .input
        .send(ProcessInput::Bytes(bytes::Bytes::from_static(b"go\n")))
        .await
        .unwrap();
    sibling.input.send(ProcessInput::End).await.unwrap();
    let mut rest = Vec::new();
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(3), sibling.output.recv())
        .await
        .unwrap()
    {
        assert!(matches!(chunk.stream, OutputStream::Stdout));
        rest.extend(chunk.bytes);
    }
    assert_eq!(rest, b"done");
    let (exit, _) = sibling.wait().await;
    assert_eq!(exit.code, Some(0), "{:?}", exit.error);
    std::mem::ManuallyDrop::into_inner(shell).shutdown_background();
}

/// Each utility runs in a job of its own, and the jobs run together: each is
/// a login shell that reads the machine's profile first.
#[cfg(unix)]
#[tokio::test]
async fn cancellation_reaps_external_programs_started_by_native_utilities() {
    use demi_runner::shell::{job::Job, scope::Scope};
    use tokio_util::sync::CancellationToken;
    let scripts = [
        "/bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60'",
        "printf x | xargs /bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60'",
        "find . -maxdepth 0 -exec /bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60' ';'",
        "find . -maxdepth 0 -exec /bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60' sh '{}' +",
        "printf x | sed 'e echo $$ > child.pid; exec /bin/sleep 60'",
    ]
    .map(|script| async move {
        let root = tempfile::tempdir().unwrap();
        let scope = Scope::new(CancellationToken::new(), None);
        let mut job = Job::start(
            script.into(),
            root.path().into(),
            crate::home(root.path()),
            false,
            scope.clone(), &demi_runner::shell::ShellRuntime::current(),
        )
            .await
        .unwrap();
        let pid = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(root.path().join("child.pid")).await
                    && let Ok(pid) = text.trim().parse::<i32>()
                {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect(script);
        job.cancel();
        let (exit, _) = tokio::time::timeout(Duration::from_secs(3), job.wait())
            .await
            .expect(script);
        assert_eq!(exit.signal.as_deref(), Some("SIGKILL"));
        assert_eq!(scope.tasks.len(), 0);
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "child {pid} survived: {script}"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    });
    futures_util::future::join_all(scripts).await;
}

#[tokio::test]
async fn job_completion_preserves_process_substitution_output() {
    use demi_runner::shell::{job::Job, scope::Scope};
    use tokio_util::sync::CancellationToken;
    let root = tempfile::tempdir().unwrap();
    let content = vec![b'x'; 256 * 1024];
    std::fs::write(root.path().join("input"), &content).unwrap();
    let mut job = Job::start(
        "cat input | tee >(sleep 0.1; cat > copied) > /dev/null".into(),
        root.path().into(),
        crate::home(root.path()),
        false,
        Scope::new(CancellationToken::new(), None), &demi_runner::shell::ShellRuntime::current(),
    )
            .await
    .unwrap();
    let (exit, _) = tokio::time::timeout(Duration::from_secs(5), job.wait())
        .await
        .unwrap();
    assert_eq!(exit.code, Some(0), "{:?}", exit.error);
    assert_eq!(std::fs::read(root.path().join("copied")).unwrap(), content);
}
