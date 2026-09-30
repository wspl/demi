//! A registration (`runner.md` § Connection and identity): the runner says
//! its protocol, logs its start, runs a job whose declared command is a
//! builtin of its shell, answers `status` while it runs, and on `drain`
//! ends its connection with a close frame and releases the installation.

use std::{collections::BTreeMap, time::Duration};

use demi_host_remote::testing::runner_binary;
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
