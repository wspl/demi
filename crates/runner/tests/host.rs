use std::{collections::BTreeMap, sync::Arc, time::Duration};

use demi_command_service::protocol::{CommandCaller, CommandContext, CommandLocale};
use demi_runner::connection::wire::{self as wire, Inbound};
use demi_runner::{host::HostServer, pipes::PipeClient};
use tokio::sync::{RwLock, mpsc};

#[tokio::test]
async fn filesystem_requests_and_kill_remain_available_during_job() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let root = tempfile::tempdir().unwrap();
        let (output, mut replies) = mpsc::channel(8);
        let pipes = PipeClient::new("http://127.0.0.1:1", Arc::new(RwLock::new(None))).unwrap();
        let host = HostServer::new(
            output,
            None,
            root.path().join("logs"),
            root.path().into(),
            BTreeMap::new(),
            pipes,
        );
        host.handle_task(
            &Inbound::JobStart {
                manifest_hash: None,
                context: context(),
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
            None,
            root.path().join("logs"),
            root.path().into(),
            device,
            pipes,
        );
        host.handle_task(
            &Inbound::JobStart {
                manifest_hash: None,
                context: context(),
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
        None,
        root.path().join("logs"),
        root.path().into(),
        BTreeMap::from([
            ("DEVICE".into(), "device".into()),
            ("OVERRIDE".into(), "device".into()),
        ]),
        pipes,
    );
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
                command: if cfg!(windows) {
                    r"C:\Windows\System32\cmd.exe".replace('\\', "/")
                } else {
                    "/usr/bin/env".into()
                },
                args: if cfg!(windows) {
                    Some(vec!["/c".into(), "set".into()])
                } else {
                    None
                },
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
        let text = String::from_utf8(stdout).unwrap();
        let values: BTreeMap<_, _> = text
            .lines()
            .filter_map(|line| line.split_once('='))
            .collect();
        assert_eq!(
            format!(
                "{}:{}",
                values.get("DEVICE").copied().unwrap_or("unset"),
                values.get("OVERRIDE").copied().unwrap_or("unset")
            ),
            expected
        );
    }
    host.close().await;
}

fn context() -> CommandContext {
    CommandContext {
        conversation: "conversation".into(),
        caller: CommandCaller::agent("node"),
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
    }
}
