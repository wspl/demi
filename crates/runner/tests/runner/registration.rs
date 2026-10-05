//! A registration (`runner.md` § Connection and identity): the runner says
//! its protocol, logs its start, runs a job whose declared command is a
//! builtin of its shell, answers `status` while it runs, and on `drain`
//! ends its connection with a close frame and releases the installation.
//! Its removal (`runner.md` § Installation, pairing and removal): a revoked
//! runner removes its installation, a refused one keeps it, and `uninstall`
//! asks the backend to revoke the device and removes the installation.

use std::{collections::BTreeMap, time::Duration};

use demi_backend_remote_host::testing::runner_binary;
use demi_runner_protocol::{
    manifest::Manifest,
    wire::{self, Inbound, Outbound},
};
use futures_util::StreamExt;
use serde_json::json;
use tokio_tungstenite::tungstenite::{Message, protocol::frame::coding::CloseCode};

use crate::{Host, context};

/// Runs `demi-runner <action>` for the installation in `state`; its exit
/// code.
async fn manage(action: &str, state: &std::path::Path) -> Option<i32> {
    tokio::process::Command::new(runner_binary())
        .arg(action)
        .env("DEMI_HOME", state)
        .env_remove("DEMI_RELEASE_ID")
        .output()
        .await
        .unwrap()
        .status
        .code()
}

#[tokio::test]
async fn backend_job_invokes_a_declared_builtin_and_drain_releases_installation() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let host = Host::start(BTreeMap::new()).await;
        let mut host = host.online().await;
        // The log answers from its files and waits for no queued line, so
        // ask until the writer has put the start there.
        loop {
            host.send(Inbound::LogRead {
                id: "log".into(),
                since: None,
                limit: 10,
                source: Some("runner".into()),
            })
            .await;
            let Outbound::LogLines { id, lines, next } = host.frame().await else {
                panic!("expected log_lines");
            };
            assert_eq!(id, "log");
            assert!(lines.iter().all(|line| line.source == "runner"));
            let started = format!("runner {} started", env!("CARGO_PKG_VERSION"));
            if lines.iter().any(|line| line.text == started) {
                assert!(next > 0);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let tree = json!({
            "name":"fixture", "summary":"Remote declaration", "kind":"rpc", "stdinField":"body",
            "input":{"type":"object", "properties":{"body":{"type":"string"}}, "required":["body"]}
        });
        let manifest = Manifest::build([serde_json::from_value(tree).unwrap()], []).unwrap();
        let hash = manifest.hash.clone();
        host.send(Inbound::Manifest {
            manifest: serde_json::to_value(manifest).unwrap(),
        })
        .await;
        // No stdin EOF is sent: --help must complete without waiting for input.
        let cwd = host.home().to_string_lossy().into_owned();
        host.send(Inbound::JobStart {
            manifest_hash: Some(hash),
            context: context(),
            job_id: "job".into(),
            script: "fixture --help && printf done".into(),
            cwd,
            env: BTreeMap::new(),
            stdin: None,
            stdout: None,
        })
        .await;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        loop {
            match host.frame().await {
                Outbound::JobOutput { stream, bytes, .. } => match stream {
                    wire::OutputStream::Stdout => stdout.extend(bytes.0),
                    wire::OutputStream::Stderr => stderr.extend(bytes.0),
                },
                Outbound::JobExit {
                    exit_code, output, ..
                } => {
                    assert_eq!(
                        exit_code,
                        Some(0),
                        "stderr={}",
                        String::from_utf8_lossy(&stderr)
                    );
                    assert_eq!(output.unwrap().stdout_bytes, stdout.len() as u64);
                    break;
                }
                other => panic!("unexpected runner reply {other:?}"),
            }
        }
        let stdout = String::from_utf8(stdout).unwrap();
        assert!(stdout.contains("fixture: Remote declaration"), "{stdout}");
        assert!(stdout.ends_with("done"), "{stdout}");

        let state = host.state();
        assert_eq!(manage("status", &state).await, Some(0));
        // The drained runner ends its connection with a close frame and waits
        // for the backend's, which this side sends as it reads on; `drain`
        // returns once the runner released the installation.
        let socket = &mut host.socket;
        let closing = async {
            let mut closing = Vec::new();
            while let Some(message) = socket.next().await {
                closing.push(message);
            }
            closing
        };
        let (drained, closing) = tokio::join!(manage("drain", &state), closing);
        assert_eq!(drained, Some(0));
        assert!(
            matches!(
                closing.as_slice(),
                [Ok(Message::Close(Some(frame)))] if frame.code == CloseCode::Away
            ),
            "{closing:?}"
        );
        while host.process.running() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!state.join("active.json").exists());
        // Without an active runner there is nobody to report.
        assert_eq!(manage("status", &state).await, Some(1));
    })
    .await
    .unwrap();
}

/// Waits until the runner process has exited, while the backend end reads
/// on and answers the close frame of the connection the runner ends.
async fn exited(host: &mut Host) {
    let socket = &mut host.socket;
    let process = &mut host.process;
    let answered = async { while socket.next().await.is_some() {} };
    let exited = async {
        while process.running() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::join!(answered, exited);
}

/// A runner whose device the user revoked hears it from its backend and
/// removes its installation (`runner.md` § Installation, pairing and
/// removal).
#[tokio::test]
async fn a_runner_whose_device_is_revoked_removes_its_installation() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        let state = host.state();
        assert!(state.join("runner-token").exists());
        host.send(Inbound::Revoked { projects: vec![] }).await;
        exited(&mut host).await;
        assert!(!state.exists(), "{}", host.process.output());
    })
    .await
    .unwrap();
}

/// A runner whose token its backend does not know may belong to a backend
/// that lost its data, so it keeps its installation: it stops and says how
/// to remove it.
#[tokio::test]
async fn a_runner_refused_at_its_hello_keeps_its_installation_and_says_how_to_remove_it() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await;
        let state = host.state();
        host.send(Inbound::HelloError {
            code: wire::HelloErrorCode::UnknownDevice,
            reason: "unknown device".into(),
        })
        .await;
        exited(&mut host).await;
        assert!(state.join("runner-token").exists());
        let output = host.process.output();
        assert!(
            output.contains("this device is no longer paired with")
                && output.contains(&format!("uninstall --home {}", state.display())),
            "{output}"
        );
    })
    .await
    .unwrap();
}

/// `uninstall` asks the active runner to remove itself: the runner asks its
/// backend to revoke the device and ends, and the installation goes, while
/// an artifact cache `DEMI_ARTIFACTS` names and another backend's
/// installation stay (`runner.md` § Installation, pairing and removal).
#[tokio::test]
async fn uninstall_removes_the_installation_and_keeps_a_shared_cache_and_other_installations() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let cache = tempfile::tempdir().unwrap();
        let env = BTreeMap::from([(
            "DEMI_ARTIFACTS".to_owned(),
            cache.path().to_string_lossy().into_owned(),
        )]);
        let mut host = Host::start(env).await.online().await;
        let state = host.state();
        std::fs::write(cache.path().join("artifact"), "shared").unwrap();
        let other = tempfile::tempdir_in(state.parent().unwrap()).unwrap();
        std::fs::write(other.path().join("runner-token"), "other\n").unwrap();

        let backend = async {
            assert!(matches!(host.frame().await, Outbound::Revoke {}));
            let projects = vec!["notes".to_owned()];
            host.send(Inbound::Revoked { projects }).await;
            // The runner ends its connection, and this end answers its close.
            while host.socket.next().await.is_some() {}
        };
        // The shell `uninstall` runs in names the shared cache too.
        let uninstall = tokio::process::Command::new(runner_binary())
            .arg("uninstall")
            .env("DEMI_HOME", &state)
            .env("DEMI_ARTIFACTS", cache.path())
            .env_remove("DEMI_RELEASE_ID")
            .output();
        let (uninstalled, ()) = tokio::join!(uninstall, backend);
        let uninstalled = uninstalled.unwrap();
        let printed = String::from_utf8_lossy(&uninstalled.stdout);
        assert!(uninstalled.status.success(), "{printed}");
        // It names the project that went with the device.
        assert!(printed.contains("files stay: notes"), "{printed}");
        assert!(!state.exists());
        assert!(cache.path().join("artifact").exists());
        assert!(other.path().join("runner-token").exists());
        exited(&mut host).await;
    })
    .await
    .unwrap();
}

/// Without an active runner, `uninstall` connects as the device itself to
/// ask the backend to revoke it, then removes the installation.
#[tokio::test]
async fn uninstall_without_an_active_runner_asks_the_backend_itself() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        let state = host.state();
        host.stop().await;

        let backend = async {
            host.reconnected().await;
            host.send(Inbound::HelloOk {
                device_id: "device".into(),
                device_name: "fixture".into(),
            })
            .await;
            assert!(matches!(host.frame().await, Outbound::Revoke {}));
            let projects = vec!["notes".to_owned()];
            host.send(Inbound::Revoked { projects }).await;
            while host.socket.next().await.is_some() {}
        };
        let uninstall = tokio::process::Command::new(runner_binary())
            .arg("uninstall")
            .env("DEMI_HOME", &state)
            .env_remove("DEMI_RELEASE_ID")
            .output();
        let (uninstalled, ()) = tokio::join!(uninstall, backend);
        let uninstalled = uninstalled.unwrap();
        let printed = String::from_utf8_lossy(&uninstalled.stdout);
        assert!(uninstalled.status.success(), "{printed}");
        assert!(printed.contains("files stay: notes"), "{printed}");
        assert!(!state.exists());
    })
    .await
    .unwrap();
}

/// `uninstall` removes a directory only when it holds an installation: one
/// that a mistaken `DEMI_HOME` names, holding something else, stays as it
/// was, and `uninstall` says why.
#[tokio::test]
async fn uninstall_leaves_a_directory_without_an_installation_as_it_was() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("notes.txt"), "mine").unwrap();
        let refused = tokio::process::Command::new(runner_binary())
            .arg("uninstall")
            .arg("--home")
            .arg(directory.path())
            .env_remove("DEMI_HOME")
            .env_remove("DEMI_RELEASE_ID")
            .output()
            .await
            .unwrap();
        let said = String::from_utf8_lossy(&refused.stderr);
        assert_eq!(refused.status.code(), Some(1), "{said}");
        assert!(said.contains("holds no runner installation"), "{said}");
        let entries: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, ["notes.txt"]);
    })
    .await
    .unwrap();
}

/// An installation in the user's home directory, or in a directory that
/// contains it, as a mistaken `DEMI_HOME` makes, loses only the runner's own
/// files: the directory stays with everything else in it (`runner.md`
/// § Installation, pairing and removal). The home is a directory the test
/// made, never the real one.
#[tokio::test]
async fn uninstall_keeps_a_home_and_its_parent_and_removes_only_the_runners_files() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("user");
        // The installation in the home itself, then in the directory that
        // contains the home.
        for (installation, kept) in [
            (home.clone(), vec!["notes.txt"]),
            (root.path().to_owned(), vec!["notes.txt", "user"]),
        ] {
            std::fs::create_dir_all(installation.join("releases/r1")).unwrap();
            std::fs::create_dir_all(installation.join("log")).unwrap();
            std::fs::write(installation.join("notes.txt"), "mine").unwrap();
            // A runner that was never paired asks no backend for a
            // revocation; the backend's port takes no connection.
            std::fs::write(
                installation.join("runner.json"),
                r#"{"backendUrl":"http://127.0.0.1:9/"}"#,
            )
            .unwrap();
            std::fs::write(installation.join("runner.log"), "").unwrap();
            let uninstalled = tokio::process::Command::new(runner_binary())
                .arg("uninstall")
                .arg("--home")
                .arg(&installation)
                .env("HOME", &home)
                .env_remove("DEMI_HOME")
                .env_remove("DEMI_RELEASE_ID")
                .output()
                .await
                .unwrap();
            let printed = String::from_utf8_lossy(&uninstalled.stdout);
            assert!(
                uninstalled.status.success(),
                "{printed}{}",
                String::from_utf8_lossy(&uninstalled.stderr)
            );
            assert!(printed.contains("which stays with everything else in it"), "{printed}");
            let mut left: Vec<_> = std::fs::read_dir(&installation)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect();
            left.sort();
            assert_eq!(left, kept, "{}", installation.display());
        }
    })
    .await
    .unwrap();
}
