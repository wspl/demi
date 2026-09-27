//! Host requests through a connection's owner: filesystem work and kills stay
//! available while a job runs, jobs and raw processes get the environment
//! `runner.md` § Host operations gives them, and a conversation's release
//! removes the output its jobs kept.

use std::{collections::BTreeMap, time::Duration};

use demi_command_service::protocol::{CommandCaller, CommandContext, CommandLocale};
use demi_runner::{
    connection::wire::{FsResult, Inbound, OutputStream, Outbound, Signal, SpawnError, SpawnErrorKind},
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
async fn a_start_the_runner_cannot_begin_ends_with_its_reason_as_the_spawn_error() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let mut host = Host::start(root.path(), BTreeMap::new()).await.online().await;
        let start = || Inbound::JobStart {
            manifest_hash: None,
            context: context(),
            job_id: "twin".into(),
            script: "sleep 60".into(),
            cwd: root.path().to_string_lossy().into_owned(),
            env: BTreeMap::new(),
            stdin: None,
            stdout: None,
        };
        // A second start under a live job's id cannot begin: its exit has no
        // status, and the runner's reason is the spawn error's detail, never
        // the signal.
        host.send(start()).await;
        host.send(start()).await;
        let exit = host.frame().await;
        assert!(
            matches!(
                &exit,
                Outbound::JobExit {
                    job_id,
                    exit_code: None,
                    signal: None,
                    spawn_error: Some(SpawnError {
                        kind: SpawnErrorKind::Other,
                        detail: Some(detail),
                    }),
                    ..
                } if job_id == "twin" && detail == "duplicate live task id"
            ),
            "{exit:?}"
        );
        // Closing ends the first job with the connection.
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn job_environment_combines_device_request_and_owned_values() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        // Neither the device nor the request names a home: the job's home is
        // the one the runner reports, whose login profile it reads with that
        // home as `$HOME`.
        std::fs::write(root.path().join(".profile"), "profile_home=\"$HOME\"\n").unwrap();
        let device = BTreeMap::from([
            ("DEVICE".into(), "device".into()),
            ("OVERRIDE".into(), "old".into()),
        ]);
        let mut host = Host::start(root.path(), device).await.online().await;
        host.send(Inbound::JobStart {
            manifest_hash: None,
            context: context(),
            job_id: "env".into(),
            script: "printf '%s:%s:%s:%s:%s' \"$DEVICE\" \"$OVERRIDE\" \"$DEMI_HOME\" \"$HOME\" \"$profile_home\""
                .into(),
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
        let state = root.path().join("state");
        let home = root.path().display();
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            format!("device:new:{}:{home}:{home}", state.display())
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

/// Runs `script` as job `job` of `conversation` and answers the file its
/// whole standard output is kept in, once the job has exited; a job that
/// does not exit answers none.
async fn job_of(host: &mut Host, conversation: &str, job: &str, script: &str, exits: bool) -> Option<std::path::PathBuf> {
    host.send(Inbound::JobStart {
        manifest_hash: None,
        context: CommandContext {
            conversation: conversation.into(),
            ..context()
        },
        job_id: job.into(),
        script: script.into(),
        cwd: host.root.to_string_lossy().into_owned(),
        env: BTreeMap::new(),
        stdin: None,
        stdout: None,
    })
    .await;
    if !exits {
        return None;
    }
    loop {
        if let Outbound::JobExit { job_id, output, .. } = host.frame().await
            && job_id == job
        {
            return Some(output.expect("a shell job keeps its output").stdout_path.into());
        }
    }
}

// About a second: three shell jobs start a login shell each.
#[tokio::test]
async fn a_release_removes_its_conversations_job_output_and_keeps_the_rest() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let mut host = Host::start(root.path(), BTreeMap::new()).await.online().await;
        let released = job_of(&mut host, "Released-1", "one", "printf done", true).await.unwrap();
        let kept = job_of(&mut host, "kept", "two", "printf kept", true).await.unwrap();
        // A job of the released conversation still runs: nothing of it goes.
        job_of(&mut host, "released-1", "live", "sleep 60", false).await;
        let jobs = root.path().join("state/jobs");
        // Every job's directory is under its conversation's, in lowercase.
        assert!(released.starts_with(jobs.join("released-1")), "{}", released.display());
        assert!(kept.starts_with(jobs.join("kept")), "{}", kept.display());

        host.send(Inbound::ConversationRelease {
            id: "release".into(),
            conversation_id: "Released-1".into(),
        })
        .await;
        loop {
            if let Outbound::ConversationReleased { id, error } = host.frame().await {
                assert_eq!((id.as_str(), error), ("release", None));
                break;
            }
        }
        assert!(!released.parent().unwrap().exists(), "the finished job's directory is gone");
        let left: Vec<_> = std::fs::read_dir(jobs.join("released-1")).unwrap().collect();
        assert_eq!(left.len(), 1, "the running job keeps its directory");
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "kept");

        host.send(Inbound::JobKill {
            job_id: "live".into(),
            signal: Some(Signal::Kill),
        })
        .await;
        host.close().await;
    })
    .await
    .unwrap();
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
