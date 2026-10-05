//! No Host request is refused for how many others are in flight
//! (`runner.md` § Load): bounded Host work waits for a slot.

use demi_runner_host::host::HostServer;
use demi_runner_process::pipes::PipeClient;
use demi_runner_protocol::wire::{self, Inbound, Outbound};
use std::{collections::BTreeMap, path::PathBuf, process::Command, time::Duration};
use tokio::sync::mpsc;

fn reply(frame: wire::Frame) -> Outbound {
    wire::decode(&frame.into_bytes()).unwrap()
}

/// Filesystem requests past the runner's concurrency wait; none answer EBUSY.
/// The test's runtime runs one task at a time, so all 500 requests are queued
/// before the first runs, and all but the runner's 32 slots wait.
#[tokio::test]
async fn filesystem_requests_wait_instead_of_failing() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file"), "x").unwrap();
        let (output, mut replies) = mpsc::channel(1024);
        let pipes = PipeClient::new(
            &"http://127.0.0.1:1".parse().unwrap(),
            tokio::sync::watch::Sender::new(None).subscribe(),
        )
        .unwrap();
        let host = HostServer::new(output, root.path().into(), pipes);
        for index in 0..500 {
            host.handle_filesystem(Inbound::FsReaddir {
                id: format!("r{index}"),
                path: ".".into(),
                cwd: None,
            })
            .unwrap();
        }
        let mut failures = BTreeMap::<String, usize>::new();
        for _ in 0..500 {
            let message = reply(replies.recv().await.unwrap());
            if !matches!(message, Outbound::FsOk(_)) {
                *failures.entry(format!("{message:?}")).or_default() += 1;
            }
        }
        assert!(failures.is_empty(), "{failures:?}");
        host.close().await;
    })
    .await
    .unwrap();
}

/// A repository with one file that is not yet committed, which its changes
/// list.
fn repository() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(dir.path()).unwrap();
    let status = Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::write(repo.join("file.txt"), "1\n").unwrap();
    (dir, repo)
}

/// Working-tree requests past the computation limit wait; none answer busy.
/// Sixteen requests arrive at once: the runner computes two at a time and
/// keeps eight directories watched.
#[tokio::test(flavor = "multi_thread")]
async fn working_tree_requests_wait_instead_of_failing() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let repositories: Vec<_> = (0..16).map(|_| repository()).collect();
        let (output, mut replies) = mpsc::channel(1024);
        let pipes = PipeClient::new(
            &"http://127.0.0.1:1".parse().unwrap(),
            tokio::sync::watch::Sender::new(None).subscribe(),
        )
        .unwrap();
        let logs = tempfile::tempdir().unwrap();
        let host = HostServer::new(output, logs.path().into(), pipes);
        for (index, (_dir, repo)) in repositories.iter().enumerate() {
            host.handle_git(Inbound::GitChanges {
                id: format!("g{index}"),
                root: repo.to_string_lossy().into_owned(),
            })
            .unwrap();
        }
        let mut failures = Vec::new();
        for _ in 0..repositories.len() {
            let message = reply(replies.recv().await.unwrap());
            if !matches!(message, Outbound::GitOk(_)) {
                failures.push(format!("{message:?}"));
            }
        }
        assert!(failures.is_empty(), "{failures:?}");
        host.close().await;
    })
    .await
    .unwrap();
}
