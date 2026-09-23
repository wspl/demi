use demi_runner::connection::wire::{RetainedOutput, Signal, WireBytes};
use demi_runner::{
    pipes::PipeClient,
    tasks::{JobConfig, JobTable, TaskCommand, TaskSpec, WorkId},
};
use std::{collections::BTreeMap, path::Path, time::Duration};
use tokio::sync::mpsc;

#[derive(serde::Deserialize)]
#[serde(tag = "type")]
enum Reply {
    #[serde(rename = "job_output")]
    Output { stream: String, bytes: WireBytes },
    #[serde(rename = "job_exit")]
    Exit {
        #[serde(rename = "exitCode")]
        exit_code: Option<f64>,
        signal: Option<String>,
        cwd: Option<String>,
        output: Option<RetainedOutput>,
    },
}

fn table(
    root: &Path,
    capacity: usize,
) -> (
    JobTable,
    mpsc::Receiver<demi_runner::connection::wire::Frame>,
) {
    let (output, receiver) = mpsc::channel(capacity);
    let token = tokio::sync::watch::Sender::new(Some("test-token".parse().unwrap())).subscribe();
    let pipes = PipeClient::new(&"http://127.0.0.1:1".parse().unwrap(), token).unwrap();
    (
        JobTable::new(JobConfig {
            output,
            output_dir: root.join("logs"),
            pipes,
            shell: demi_runner::shell::ShellRuntime::current(),
            commands: None,
        }),
        receiver,
    )
}

/// Waits for a shell job's complete readiness marker across output frames.
async fn wait_for_job_ready(job: &mut demi_runner::shell::job::Job) {
    tokio::time::timeout(Duration::from_secs(3), async {
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

#[tokio::test]
async fn shell_job_keeps_full_logs_but_only_sends_head_and_tail_views() {
    let root = tempfile::tempdir().unwrap();
    let (mut table, mut receiver) = table(root.path(), 8);
    table
        .start(TaskSpec {
            id: "job".into(),
            cwd: root.path().into(),
            env: BTreeMap::new(),
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
            Reply::Output { stream, bytes } => {
                if stream == "stdout" {
                    stdout.extend(bytes.0);
                } else {
                    stderr.extend(bytes.0);
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
                assert_eq!(output.stdout_tail.0.len(), 32768);
                assert_eq!(output.stderr_tail.0.last(), Some(&b'1'));
                assert_eq!(std::fs::read(output.stdout_path).unwrap().len(), 60000);
                assert_eq!(std::fs::read(output.stderr_path).unwrap().len(), 40000);
                break;
            }
        }
    }
    table.close().await;
    assert_eq!(table.len(), 0);
}

#[tokio::test]
async fn functions_and_compound_pipelines_drain_large_output_and_here_documents() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("input"), vec![b'x'; 262_144]).unwrap();
    let (mut table, mut receiver) = table(root.path(), 8);
    table
        .start(TaskSpec {
            id: "pipeline".into(),
            cwd: root.path().into(),
            env: BTreeMap::new(),
            command: TaskCommand::Shell {
                script: "producer() { cat input; }; value=$(producer | cat | cat); printf '%s\\n' \"${#value}\"; { producer; } | wc -c; (producer) | wc -c; cat <<EOF | wc -c\n$value\nEOF\ncat <<< \"$value\" | wc -c".into(),
                stdin: None,
                stdout: None,
                commands: None,
            },
        })
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        let mut output = Vec::new();
        loop {
            let message = receiver.recv().await.unwrap();
            match rmp_serde::from_slice::<Reply>(&message.into_bytes()).unwrap() {
                Reply::Output { stream, bytes } => {
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
    let (mut table, mut receiver) = table(root.path(), 8);
    table
        .start(TaskSpec {
            id: "job".into(),
            cwd: root.path().into(),
            env: BTreeMap::new(),
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
    tokio::time::timeout(Duration::from_secs(3), async {
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

#[tokio::test]
async fn shell_cancellation_reports_the_requesting_signal() {
    use demi_runner::shell::{job::Job, scope::Scope};
    use tokio_util::sync::CancellationToken;

    for signal in [
        Some(Signal::Terminate),
        Some(Signal::Interrupt),
        Some(Signal::Hangup),
        Some(Signal::Quit),
        Some(Signal::Kill),
        None,
    ] {
        let root = tempfile::tempdir().unwrap();
        let scope = Scope::new(CancellationToken::new(), None);
        let mut job = Job::start(
            "printf ready; sleep 60".into(),
            root.path().into(),
            BTreeMap::new(),
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
        assert!(exit.error.is_none());
        assert_eq!(scope.tasks.len(), 0);
    }
}

#[tokio::test]
async fn shutdown_does_not_wait_for_a_blocked_output_consumer() {
    let root = tempfile::tempdir().unwrap();
    let (mut table, mut receiver) = table(root.path(), 1);
    table
        .start(TaskSpec {
            id: "spawn".into(),
            cwd: root.path().into(),
            env: BTreeMap::new(),
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

#[tokio::test]
async fn jobs_share_the_runner_process_and_cancellation_is_isolated() {
    use demi_runner::shell::{job::Job, scope::Scope};
    use tokio_util::sync::CancellationToken;
    for script in [
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
    ] {
        let root = tempfile::tempdir().unwrap();
        let scope = Scope::new(CancellationToken::new(), None);
        let mut blocked = Job::start(
            format!("printf ready; {script}"),
            root.path().into(),
            BTreeMap::new(),
            true,
            scope.clone(), &demi_runner::shell::ShellRuntime::current(),
        )
            .await
        .unwrap();
        wait_for_job_ready(&mut blocked).await;
        let mut sibling = Job::start(
            "printf '%s' $$; sleep 0.1; printf done".into(),
            root.path().into(),
            BTreeMap::new(),
            false,
            Scope::new(CancellationToken::new(), None), &demi_runner::shell::ShellRuntime::current(),
        )
            .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        blocked.cancel();
        let (exit, _) = tokio::time::timeout(Duration::from_secs(3), blocked.wait())
            .await
            .expect(script);
        assert_eq!(exit.signal.as_deref(), Some("SIGKILL"), "{script}");
        assert_eq!(scope.tasks.len(), 0, "{script}");
        let mut output = Vec::new();
        while let Some(chunk) = tokio::time::timeout(Duration::from_secs(3), sibling.output.recv())
            .await
            .unwrap()
        {
            assert!(matches!(
                chunk.stream,
                demi_runner::process::OutputStream::Stdout
            ));
            output.extend(chunk.bytes);
        }
        let (exit, _) = sibling.wait().await;
        assert_eq!(exit.code, Some(0), "{:?}", exit.error);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("{}done", std::process::id())
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_reaps_external_programs_started_by_native_utilities() {
    use demi_runner::shell::{job::Job, scope::Scope};
    use tokio_util::sync::CancellationToken;
    for script in [
        "/bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60'",
        "printf x | xargs /bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60'",
        "find . -maxdepth 0 -exec /bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60' ';'",
        "find . -maxdepth 0 -exec /bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60' sh '{}' +",
        "printf x | sed 'e echo $$ > child.pid; exec /bin/sleep 60'",
    ] {
        let root = tempfile::tempdir().unwrap();
        let scope = Scope::new(CancellationToken::new(), None);
        let mut job = Job::start(
            script.into(),
            root.path().into(),
            BTreeMap::new(),
            false,
            scope.clone(), &demi_runner::shell::ShellRuntime::current(),
        )
            .await
        .unwrap();
        let pid = tokio::time::timeout(Duration::from_secs(3), async {
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
    }
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
        BTreeMap::new(),
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
