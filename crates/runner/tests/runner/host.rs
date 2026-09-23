//! Host requests through a connection's owner: filesystem work and kills stay
//! available while a job runs, and jobs and raw processes get the environment
//! `runner.md` § Host operations gives them.

use std::{collections::BTreeMap, time::Duration};

use demi_command_service::protocol::{CommandCaller, CommandContext, CommandLocale};
use demi_runner::{
    connection::wire::{FsResult, Inbound, OutputStream, Outbound, Signal},
    testing::Host,
};

#[tokio::test]
async fn filesystem_requests_and_kill_remain_available_during_job() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let mut host = Host::start(root.path(), BTreeMap::new()).await.online().await;
        host.send(Inbound::JobStart {
            manifest_hash: None,
            context: context(),
            job_id: "live".into(),
            script: "sleep 60".into(),
            cwd: root.path().to_string_lossy().into_owned(),
            env: BTreeMap::new(),
            stdin: None,
            stdout: None,
        })
        .await;
        host.send(Inbound::FsExists {
            id: "file".into(),
            path: ".".into(),
            cwd: None,
        })
        .await;
        let reply = host.frame().await;
        assert!(
            matches!(&reply, Outbound::FsOk(ok) if ok.id == "file" && ok.result == FsResult::Exists(true)),
            "{reply:?}"
        );
        host.send(Inbound::JobKill {
            job_id: "live".into(),
            signal: Some(Signal::Kill),
        })
        .await;
        let exit = host.frame().await;
        assert!(
            matches!(&exit, Outbound::JobExit { job_id, .. } if job_id == "live"),
            "{exit:?}"
        );
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn job_environment_combines_device_request_and_owned_values() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let device = BTreeMap::from([
            ("DEVICE".into(), "device".into()),
            ("OVERRIDE".into(), "old".into()),
        ]);
        let mut host = Host::start(root.path(), device).await.online().await;
        host.send(Inbound::JobStart {
            manifest_hash: None,
            context: context(),
            job_id: "env".into(),
            script: "printf '%s:%s:%s' \"$DEVICE\" \"$OVERRIDE\" \"$DEMI_HOME\"".into(),
            cwd: root.path().to_string_lossy().into_owned(),
            env: BTreeMap::from([
                ("OVERRIDE".into(), "new".into()),
                ("DEMI_HOME".into(), "untrusted".into()),
            ]),
            stdin: None,
            stdout: None,
        })
        .await;
        let mut stdout = Vec::new();
        loop {
            match host.frame().await {
                Outbound::JobExit { .. } => break,
                Outbound::JobOutput { bytes, .. } => stdout.extend(bytes.0),
                other => panic!("unexpected {other:?}"),
            }
        }
        let home = root.path().join("state");
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            format!("device:new:{}", home.display())
        );
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn raw_spawn_inherits_environment_only_when_requested() {
    let root = tempfile::tempdir().unwrap();
    let device = BTreeMap::from([
        ("DEVICE".into(), "device".into()),
        ("OVERRIDE".into(), "device".into()),
    ]);
    let mut host = Host::start(root.path(), device).await.online().await;
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
        host.send(Inbound::Spawn {
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
        })
        .await;
        let mut stdout = Vec::new();
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(5), host.frame())
                .await
                .unwrap();
            match frame {
                Outbound::SpawnExit { exit_code, .. } => {
                    assert_eq!(exit_code, Some(0));
                    break;
                }
                Outbound::SpawnOutput {
                    stream: OutputStream::Stdout,
                    bytes,
                    ..
                } => stdout.extend(bytes.0),
                _ => {}
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
