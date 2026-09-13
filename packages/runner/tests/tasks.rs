use demi_runner::{
    pipes::PipeClient,
    tasks::{ShellJob, TaskCommand, TaskKind, TaskSpec, TaskTable},
};
use demi_runner_protocol::{JobExitOutput, WireBytes};
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};
use tokio::sync::{RwLock, mpsc};

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
        output: Option<JobExitOutput>,
    },
}

fn table(
    root: &Path,
    capacity: usize,
) -> (TaskTable, mpsc::Receiver<demi_runner_protocol::Outbound>) {
    let (output, receiver) = mpsc::channel(capacity);
    let token = Arc::new(RwLock::new(Some("test-token".into())));
    let pipes = PipeClient::new("http://127.0.0.1:1", token).unwrap();
    (
        TaskTable::new(
            output,
            env!("CARGO_BIN_EXE_demi-runner").into(),
            root.join("logs"),
            pipes,
        ),
        receiver,
    )
}

#[tokio::test]
async fn shell_job_keeps_full_logs_but_only_sends_head_and_tail_views() {
    let root = tempfile::tempdir().unwrap();
    let (table, mut receiver) = table(root.path(), 8);
    table
        .start(TaskSpec {
            lifetime: None,
            id: "job".into(),
            cwd: root.path().into(),
            env: BTreeMap::new(),
            command: TaskCommand::Shell {
                script: "printf '%060000d' 0; printf '%040000d' 1 >&2; mkdir child; cd child"
                    .into(),
                stdin: None,
                stdout: None,
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
                    cwd.unwrap(),
                    root.path()
                        .join("child")
                        .canonicalize()
                        .unwrap()
                        .to_string_lossy()
                );
                let output = output.unwrap();
                assert_eq!(stdout.len(), 32768);
                assert_eq!(stderr.len(), 32768);
                assert_eq!(output.stdout_bytes, 60000.0);
                assert_eq!(output.stderr_bytes, 40000.0);
                assert_eq!(output.stdout_tail.0.len(), 32768);
                assert_eq!(output.stderr_tail.0.last(), Some(&b'1'));
                assert_eq!(std::fs::read(output.stdout_path).unwrap().len(), 60000);
                assert_eq!(std::fs::read(output.stderr_path).unwrap().len(), 40000);
                break;
            }
        }
    }
    table.close().await;
    assert_eq!(table.count(), 0);
}

#[tokio::test]
async fn cancellation_terminates_a_blocking_native_builtin() {
    let root = tempfile::tempdir().unwrap();
    let (table, mut receiver) = table(root.path(), 8);
    table
        .start(TaskSpec {
            lifetime: None,
            id: "job".into(),
            cwd: root.path().into(),
            env: BTreeMap::new(),
            command: TaskCommand::Shell {
                script: "printf ready; sleep 60".into(),
                stdin: None,
                stdout: None,
            },
        })
        .unwrap();
    let message = receiver.recv().await.unwrap();
    assert!(matches!(
        rmp_serde::from_slice::<Reply>(&message.into_bytes()).unwrap(),
        Reply::Output { .. }
    ));
    table
        .signal(TaskKind::Job, "job", "SIGKILL".into())
        .unwrap();
    let message = tokio::time::timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        rmp_serde::from_slice::<Reply>(&message.into_bytes()).unwrap(),
        Reply::Exit { .. }
    ));
    table.close().await;
    assert_eq!(table.count(), 0);
}

#[tokio::test]
async fn shutdown_does_not_wait_for_a_blocked_output_consumer() {
    let root = tempfile::tempdir().unwrap();
    let job = root.path().join("job.json");
    std::fs::write(
        &job,
        serde_json::to_vec(&ShellJob {
            login: false,
            live: false,
            script: "while :; do printf '%04096d' 0; done".into(),
            cwd_file: root.path().join("cwd"),
        })
        .unwrap(),
    )
    .unwrap();
    let (table, mut receiver) = table(root.path(), 1);
    table
        .start(TaskSpec {
            lifetime: None,
            id: "spawn".into(),
            cwd: root.path().into(),
            env: BTreeMap::new(),
            command: TaskCommand::Process {
                command: env!("CARGO_BIN_EXE_demi-runner").into(),
                args: vec!["shell-job".into(), job.to_string_lossy().into_owned()],
                process_group: true,
            },
        })
        .unwrap();
    receiver.recv().await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), table.close())
        .await
        .unwrap();
    assert_eq!(table.count(), 0);
}
