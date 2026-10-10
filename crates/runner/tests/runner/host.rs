//! Host requests through a connection's owner: filesystem work and kills stay
//! available while a job runs, jobs and raw processes get the environment
//! `runner.md` § Host operations gives them, the device's without the
//! runner's own names, and a job's directory lasts
//! until the backend has what it needs of the job.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use demi_runner_protocol::wire::{
    self, FsResult, Inbound, Outbound, OutputStream, Signal, SpawnError, SpawnErrorKind,
};

use crate::{Host, context, start_job};

#[tokio::test]
async fn filesystem_requests_and_kill_remain_available_during_job() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        start_job(&mut host, "live", "sleep 60").await;
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
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        // A second start under a live job's id cannot begin: its exit has no
        // status, and the runner's reason is the spawn error's detail, never
        // the signal.
        start_job(&mut host, "twin", "sleep 60").await;
        start_job(&mut host, "twin", "sleep 60").await;
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
        // Stopping ends the first job with the connection.
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn job_environment_combines_device_request_and_owned_values() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let device = BTreeMap::from([
            ("DEVICE".into(), "device".into()),
            ("OVERRIDE".into(), "old".into()),
        ]);
        let mut host = Host::start(device).await.online().await;
        // The job's home is the device's, whose login profile it reads with
        // that home as `$HOME`; the request cannot name another installation.
        std::fs::write(host.home().join(".profile"), "profile_home=\"$HOME\"\n").unwrap();
        let cwd = host.home().to_string_lossy().into_owned();
        host.send(Inbound::JobStart {
            manifest_hash: None,
            viewable: None,
            context: context(),
            job_id: "env".into(),
            script: "printf '%s:%s:%s:%s:%s' \"$DEVICE\" \"$OVERRIDE\" \"$DEMI_HOME\" \"$HOME\" \"$profile_home\""
                .into(),
            cwd,
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
        let home = host.home().display().to_string();
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            format!("device:new:{}:{home}:{home}", host.state().display())
        );
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn raw_spawn_inherits_environment_only_when_requested() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let device = BTreeMap::from([
            ("DEVICE".into(), "device".into()),
            ("OVERRIDE".into(), "device".into()),
        ]);
        let mut host = Host::start(device).await.online().await;
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
                descriptors: None,
            })
            .await;
            let mut stdout = Vec::new();
            loop {
                match host.frame().await {
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
    })
    .await
    .unwrap();
}

/// The device environment is the runner's own without its `DEMI_*` names,
/// for a job and a raw process alike; and a job's temporary directory is the
/// device's, never one under the job root, whose path is too long for a
/// dev server's Unix socket on macOS (`runner.md` § Host operations, § Shell
/// jobs). Before the fix, the job read the runner's release and setting and
/// a `TMPDIR` under its job directory.
#[cfg(unix)]
#[tokio::test]
async fn the_device_environment_leaves_out_the_runners_own_names_and_keeps_its_temporary_directory()
{
    tokio::time::timeout(Duration::from_secs(60), async {
        let device = BTreeMap::from([
            ("DEMI_RELEASE_ID".into(), "runner-release".into()),
            ("DEMI_SETTING".into(), "developer".into()),
        ]);
        let mut host = Host::start(device).await.online().await;
        start_job(
            &mut host,
            "job",
            r#"printf '%s\n' "${DEMI_RELEASE_ID-unset}" "${DEMI_SETTING-unset}" "${TMPDIR-unset}""#,
        )
        .await;
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        loop {
            match host.frame().await {
                Outbound::JobExit { exit_code, .. } => {
                    assert_eq!(exit_code, Some(0), "{}", String::from_utf8_lossy(&stderr));
                    break;
                }
                Outbound::JobOutput {
                    stream: OutputStream::Stdout,
                    bytes,
                    ..
                } => stdout.extend(bytes.0),
                Outbound::JobOutput { bytes, .. } => stderr.extend(bytes.0),
                other => panic!("unexpected {other:?}"),
            }
        }
        // The harness names the runner's temporary directory in its state.
        let temporary = host.state().join("tmp");
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            format!("unset\nunset\n{}\n", temporary.display())
        );

        host.send(Inbound::Spawn {
            spawn_id: "process".into(),
            command: "/usr/bin/env".into(),
            args: None,
            cwd: None,
            env: None,
            inherit_env: None,
            kill_process_group: Some(true),
            descriptors: None,
        })
        .await;
        let mut stdout = Vec::new();
        loop {
            match host.frame().await {
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
        let names: Vec<_> = text
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name))
            .filter(|name| matches!(*name, "DEMI_RELEASE_ID" | "DEMI_SETTING"))
            .collect();
        assert!(names.is_empty(), "{text}");
        host.close().await;
    })
    .await
    .unwrap();
}

/// A process reads the bytes its descriptor carries, and then the pipe's
/// end, and the bytes appear nowhere else: a token a provider's process
/// reads from a descriptor (`claude-code.md` § Accounts and sign-in).
#[cfg(unix)]
#[tokio::test]
async fn a_raw_spawn_reads_its_descriptors_bytes_and_their_end() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        host.send(Inbound::Spawn {
            spawn_id: "descriptor".into(),
            command: "/bin/sh".into(),
            args: Some(vec![
                "-c".into(),
                "cat <&3; echo; echo \"env:$(env | grep -c token-bytes)\"".into(),
            ]),
            cwd: None,
            env: None,
            inherit_env: None,
            kill_process_group: Some(true),
            descriptors: Some(vec![wire::Descriptor {
                fd: 3,
                bytes: wire::WireBytes(b"token-bytes".to_vec()),
            }]),
        })
        .await;
        let mut stdout = Vec::new();
        loop {
            match host.frame().await {
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
        assert_eq!(String::from_utf8(stdout).unwrap(), "token-bytes\nenv:0\n");
        host.close().await;
    })
    .await
    .unwrap();
}

/// Starts `script` as job `job`; for one that `exits`, waits for its end.
async fn job_of(host: &mut Host, job: &str, script: &str, exits: bool) {
    start_job(host, job, script).await;
    if !exits {
        return;
    }
    loop {
        if let Outbound::JobExit { job_id, .. } = host.frame().await
            && job_id == job
        {
            return;
        }
    }
}

/// The job directories under the runner's job root.
fn directories(host: &Host) -> Vec<PathBuf> {
    let mut found: Vec<_> = std::fs::read_dir(host.state().join("jobs"))
        .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default();
    found.retain(|path| path.is_dir());
    found.sort();
    found
}

/// Waits until the runner holds `count` job directories.
async fn directories_become(host: &Host, count: usize) {
    while directories(host).len() != count {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// A job's directory lasts until the backend releases the job, which keeps
/// a running job's; the connection's end removes every one, and a runner
/// that starts removes what an earlier one left (`runner.md` § Pipes and
/// output). About two seconds: the runner starts twice, and two shell jobs
/// start a login shell each.
#[tokio::test]
async fn job_directories_go_with_their_release_the_connection_and_the_next_start() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await;
        host.stop().await;
        let left = host.state().join("jobs/job-left/output");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::write(left.join("head"), b"what a runner that ended left").unwrap();
        host.restart().await;
        let mut host = host.online().await;
        assert_eq!(directories(&host), Vec::<PathBuf>::new());

        job_of(&mut host, "one", "printf done", true).await;
        job_of(&mut host, "live", "sleep 60", false).await;
        directories_become(&host, 2).await;
        host.send(Inbound::JobRelease {
            job_id: "one".into(),
        })
        .await;
        host.send(Inbound::JobRelease {
            job_id: "live".into(),
        })
        .await;
        directories_become(&host, 1).await;

        host.stop().await;
        assert_eq!(directories(&host), Vec::<PathBuf>::new());
    })
    .await
    .unwrap();
}
