use std::{collections::BTreeMap, sync::Arc, time::Duration};

use demi_runner::{host::HostServer, pipes::PipeClient};
use demi_runner_protocol::{self as wire, Inbound};
use tokio::sync::{RwLock, mpsc};

#[tokio::test]
async fn filesystem_requests_and_kill_remain_available_during_job() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let root = tempfile::tempdir().unwrap();
        let (output, mut replies) = mpsc::channel(8);
        let pipes = PipeClient::new("http://127.0.0.1:1", Arc::new(RwLock::new(None))).unwrap();
        let host = HostServer::new(
            output,
            env!("CARGO_BIN_EXE_demi-runner").into(),
            root.path().join("logs"),
            root.path().into(),
            BTreeMap::new(),
            pipes,
        );
        host.handle_task(
            &Inbound::JobStart {
                manifest_hash: None,
                job_id: "live".into(),
                script: "sleep 60".into(),
                cwd: root.path().to_string_lossy().into_owned(),
                env: BTreeMap::new(),
                stdin: None,
                stdout: None,
            },
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        host.handle_filesystem(Inbound::FsExists {
            id: "file".into(),
            path: ".".into(),
            cwd: None,
        })
        .unwrap();
        let reply: serde_json::Value =
            rmp_serde::from_slice(&replies.recv().await.unwrap().into_bytes()).unwrap();
        assert_eq!(
            reply,
            serde_json::json!({"type":"fs_ok", "id":"file", "op":"exists", "result":true})
        );
        host.handle_task(
            &Inbound::JobKill {
                job_id: "live".into(),
                signal: Some("SIGKILL".into()),
            },
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        #[derive(serde::Deserialize)]
        struct Exit {
            #[serde(rename = "type")]
            kind: String,
            #[serde(rename = "jobId")]
            id: String,
        }
        let exit: Exit =
            rmp_serde::from_slice(&replies.recv().await.unwrap().into_bytes()).unwrap();
        assert_eq!(exit.kind, "job_exit");
        assert_eq!(exit.id, "live");
        host.close().await;
        assert_eq!(host.tasks.count(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn job_environment_combines_device_request_and_owned_context() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let root = tempfile::tempdir().unwrap();
        let (output, mut replies) = mpsc::channel(8);
        let pipes = PipeClient::new("http://127.0.0.1:1", Arc::new(RwLock::new(None))).unwrap();
        let device = BTreeMap::from([
            ("DEVICE".into(), "device".into()),
            ("OVERRIDE".into(), "old".into()),
        ]);
        let host = HostServer::new(
            output,
            env!("CARGO_BIN_EXE_demi-runner").into(),
            root.path().join("logs"),
            root.path().into(),
            device,
            pipes,
        );
        host.handle_task(
            &Inbound::JobStart {
                manifest_hash: None,
                job_id: "env".into(),
                script: "printf '%s:%s:%s' \"$DEVICE\" \"$OVERRIDE\" \"$CONTEXT\"".into(),
                cwd: root.path().to_string_lossy().into_owned(),
                env: BTreeMap::from([
                    ("OVERRIDE".into(), "new".into()),
                    ("CONTEXT".into(), "untrusted".into()),
                ]),
                stdin: None,
                stdout: None,
            },
            &BTreeMap::from([("CONTEXT".into(), "owned".into())]),
            None,
        )
        .unwrap();
        #[derive(serde::Deserialize)]
        struct Reply {
            #[serde(rename = "type")]
            kind: String,
            bytes: Option<wire::WireBytes>,
        }
        let mut stdout = Vec::new();
        loop {
            let reply: Reply =
                rmp_serde::from_slice(&replies.recv().await.unwrap().into_bytes()).unwrap();
            if reply.kind == "job_exit" {
                break;
            }
            stdout.extend(reply.bytes.unwrap().0);
        }
        assert_eq!(stdout, b"device:new:owned");
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn raw_spawn_inherits_environment_only_when_requested() {
    let root = tempfile::tempdir().unwrap();
    let (output, mut replies) = mpsc::channel(8);
    let pipes = PipeClient::new("http://127.0.0.1:1", Arc::new(RwLock::new(None))).unwrap();
    let host = HostServer::new(
        output,
        env!("CARGO_BIN_EXE_demi-runner").into(),
        root.path().join("logs"),
        root.path().into(),
        BTreeMap::from([
            ("DEVICE".into(), "device".into()),
            ("OVERRIDE".into(), "device".into()),
        ]),
        pipes,
    );
    let job_path = root.path().join("job.json");
    std::fs::write(
        &job_path,
        serde_json::to_vec(&demi_runner::tasks::ShellJob {
            login: false,
            live: false,
            script: "printf '%s:%s' \"${DEVICE-unset}\" \"${OVERRIDE-unset}\"".into(),
            cwd_file: root.path().join("cwd"),
        })
        .unwrap(),
    )
    .unwrap();
    for (id, inherit_env, env, expected) in [
        ("default", None, None, "device:device"),
        (
            "replace",
            None,
            Some(BTreeMap::from([("OVERRIDE".into(), Some("caller".into()))])),
            "unset:caller",
        ),
        (
            "extend",
            Some(true),
            Some(BTreeMap::from([("OVERRIDE".into(), Some("caller".into()))])),
            "device:caller",
        ),
        (
            "remove",
            Some(true),
            Some(BTreeMap::from([("DEVICE".into(), None)])),
            "unset:device",
        ),
    ] {
        host.handle_task(
            &Inbound::Spawn {
                spawn_id: id.into(),
                command: env!("CARGO_BIN_EXE_demi-runner").into(),
                args: Some(vec![
                    "shell-job".into(),
                    job_path.to_string_lossy().into_owned(),
                ]),
                cwd: None,
                env,
                inherit_env,
                kill_process_group: Some(true),
            },
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        let mut stdout = Vec::new();
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(5), replies.recv())
                .await
                .unwrap()
                .unwrap();
            #[derive(serde::Deserialize)]
            struct Reply {
                #[serde(rename = "type")]
                kind: String,
                #[serde(rename = "exitCode")]
                exit_code: Option<f64>,
                stream: Option<String>,
                bytes: Option<wire::WireBytes>,
            }
            let reply: Reply = rmp_serde::from_slice(&frame.into_bytes()).unwrap();
            if reply.kind == "spawn_exit" {
                assert_eq!(reply.exit_code, Some(0.0));
                break;
            }
            if reply.kind == "spawn_output" && reply.stream.as_deref() == Some("stdout") {
                stdout.extend(reply.bytes.unwrap().0);
            }
        }
        assert_eq!(stdout, expected.as_bytes());
    }
    host.close().await;
}
