#![cfg(feature = "test-fixtures")]
//! No request the runner serves is refused for how many others are in flight
//! (`runner.md` § Load): native calls and commands the backend implements are
//! not counted, and bounded Host work waits for a slot.

use demi_command_service::protocol::{
    CommandCaller, CommandContext, CommandLocale, Invocation, LocalInvocation, PackageArtifact,
    PackageDescriptor, Record,
};
use demi_runner::{
    commands::command_client::{RawCommand, Stdio, forward},
    connection::wire::Inbound,
    host::HostServer,
    pipes::PipeClient,
    services::{
        ArtifactResolver, ArtifactSource, Resident, RuntimeError, ServiceLease, ServiceRegistry,
        target,
    },
    testing::Dispatch,
};
use futures_util::future::BoxFuture;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct Local(PathBuf);
impl ArtifactResolver for Local {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async { Ok(ArtifactSource::Local(self.0.clone())) })
    }
}

fn command_context(conversation: &str) -> CommandContext {
    CommandContext {
        conversation: conversation.into(),
        caller: CommandCaller::agent("node"),
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
    }
}

fn invocation(operation: &str, conversation: &str) -> Invocation {
    Invocation {
        context: command_context(conversation),
        operation: operation.into(),
        invocation_id: operation.into(),
        args: json!({}),
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        env: BTreeMap::new(),
        edits: None,
        json: None,
    }
}

/// A running fixture service and the lease that keeps it, as a job's context
/// would.
async fn resident(root: &Path) -> (ServiceRegistry, ServiceLease, Resident) {
    let services = ServiceRegistry::new(root.join("cache"), root.into(), BTreeMap::new())
        .await
        .unwrap();
    let bytes = tokio::fs::read(env!("CARGO_BIN_EXE_demi-native-fixture"))
        .await
        .unwrap();
    let path = root.join("fixture");
    tokio::fs::write(&path, &bytes).await.unwrap();
    let descriptor = PackageDescriptor {
        id: "fixture".into(),
        version: "1.0.0".into(),
        protocol_version: 1,
        operations: ["where", "echo", "first", "spin", "result", "retain", "crash"]
            .map(String::from)
            .to_vec(),
        targets: BTreeMap::from([(
            target().into(),
            PackageArtifact {
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                size: bytes.len() as u64,
            },
        )]),
    };
    let lease = services
        .handle()
        .lease(descriptor.targets[target()].sha256.clone()).await;
    let resident = services
        .handle()
        .acquire(
            &descriptor,
            Arc::new(Local(path)),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    (services, lease, resident)
}

/// Native calls held open, then a cancel and a conversation release at once:
/// the release is admitted, and every other call keeps running.
#[tokio::test(flavor = "multi_thread")]
async fn native_calls_are_never_turned_away() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let (services, _lease, resident) = resident(root.path()).await;
        let client = resident.client();
        let mut held = Vec::new();
        for _ in 0..128 {
            held.push(client.invoke(&invocation("echo", "running")).await.unwrap());
        }
        for attempt in 0..40 {
            let (mut input, _output) = held.remove(0);
            input.cancel();
            services
                .handle()
                .release_conversation(&format!("archived-{attempt}"))
                .await
                .unwrap();
            held.push(client.invoke(&invocation("echo", "running")).await.unwrap());
        }
        for (mut input, mut output) in held {
            input.end().unwrap();
            let mut completed = false;
            while let Some(record) = output.next().await.unwrap() {
                completed |= matches!(record, Record::Completion(value) if value.exit_code == 0);
            }
            assert!(completed);
        }
        assert!(client.info().await.is_ok());
        services.close().await;
    })
    .await
    .unwrap();
}

/// Backend-implemented commands are relayed however many are in flight.
#[tokio::test(flavor = "multi_thread")]
async fn backend_commands_are_never_turned_away() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path().to_owned();
        let tree = json!({
            "name": "fixture", "summary": "Test callback.", "kind": "rpc", "runningHint": "Working",
            "input": {"type": "object", "properties": {"body": {"type": "string"}}, "required": ["body"]}, "stdinField": "body"
        });
        let manifest = demi_runner_protocol::manifest::Manifest::build(
            [serde_json::from_value(tree).unwrap()],
            [],
        )
        .unwrap();
        let manifest = serde_json::to_value(manifest).unwrap();
        let pipes = PipeClient::new(&"http://127.0.0.1:1".parse().unwrap(), tokio::sync::watch::Sender::new(None).subscribe()).unwrap();
        let mut dispatch = Dispatch::new(&cwd, manifest, pipes).await;
        let mut outgoing = std::mem::replace(&mut dispatch.outgoing, mpsc::channel(1).1);
        // The backend never answers; count the calls that reach it.
        let reached = Arc::new(AtomicUsize::new(0));
        let counted = reached.clone();
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                let value: serde_json::Value =
                    rmp_serde::from_slice(&message.into_bytes()).unwrap();
                if value["type"] == "rpc_call" {
                    counted.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        let (context, _guard) = dispatch.context("job", command_context("conversation")).await;
        let request = LocalInvocation {
            operation: "raw".into(),
            invocation_id: "raw".into(),
            args: serde_json::to_value(RawCommand {
                context: context.id.clone(),
                root: "fixture".into(),
                argv: vec![],
                live: true,
            })
            .unwrap(),
            cwd: cwd.to_string_lossy().into_owned(),
            env: BTreeMap::new(),
        };
        let cancel = CancellationToken::new();
        let mut running = tokio::task::JoinSet::new();
        for _ in 0..200 {
            let endpoint = dispatch.server.endpoint().to_owned();
            let request = request.clone();
            let cancel = cancel.clone();
            running.spawn(async move {
                let mut stderr = Vec::new();
                let result = forward(
                    &endpoint,
                    &request,
                    Stdio {
                        stdin: tokio::io::empty(),
                        stdout: tokio::io::sink(),
                        stderr: &mut stderr,
                    },
                    cancel,
                )
                .await;
                (result.map(|completion| completion.exit_code).map_err(|error| error.to_string()), stderr)
            });
        }
        while reached.load(Ordering::SeqCst) < 200 {
            if let Some(result) = running.try_join_next() {
                let (exit, stderr) = result.unwrap();
                panic!(
                    "a call ended while the backend held it: {exit:?} {}",
                    String::from_utf8_lossy(&stderr)
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cancel.cancel();
        while running.join_next().await.is_some() {}
        dispatch.close().await;
    })
    .await
    .unwrap();
}

fn reply(bytes: Vec<u8>) -> serde_json::Value {
    rmp_serde::from_slice(&bytes).unwrap()
}

/// Filesystem requests past the runner's concurrency wait; none answer EBUSY.
#[tokio::test(flavor = "multi_thread")]
async fn filesystem_requests_wait_instead_of_failing() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        for index in 0..200 {
            std::fs::write(root.path().join(format!("f{index}")), "x").unwrap();
        }
        let (output, mut replies) = mpsc::channel(1024);
        let pipes = PipeClient::new(&"http://127.0.0.1:1".parse().unwrap(), tokio::sync::watch::Sender::new(None).subscribe()).unwrap();
        let host = HostServer::new(output, root.path().into(), pipes);
        for index in 0..500 {
            host.handle_filesystem(Inbound::FsReaddir {
                id: format!("r{index}"),
                path: ".".into(),
                cwd: None,
                with_file_types: Some(true),
            })
            .unwrap();
        }
        let mut failures = BTreeMap::<String, usize>::new();
        for _ in 0..500 {
            let value = reply(replies.recv().await.unwrap().into_bytes());
            if value["type"] != "fs_ok" {
                *failures.entry(value.to_string()).or_default() += 1;
            }
        }
        assert!(failures.is_empty(), "{failures:?}");
        host.close().await;
    })
    .await
    .unwrap();
}

fn repository() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(dir.path()).unwrap();
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .unwrap();
        assert!(status.success());
    };
    git(&["init", "-q", "-b", "main"]);
    for index in 0..300 {
        std::fs::write(repo.join(format!("f{index}.txt")), "1\n").unwrap();
    }
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "first"]);
    for index in 0..300 {
        std::fs::write(repo.join(format!("f{index}.txt")), "2\n").unwrap();
    }
    (dir, repo)
}

/// Working-tree requests past the computation limit wait; none answer busy.
#[tokio::test(flavor = "multi_thread")]
async fn working_tree_requests_wait_instead_of_failing() {
    tokio::time::timeout(Duration::from_secs(120), async {
        let repositories: Vec<_> = (0..16).map(|_| repository()).collect();
        let (output, mut replies) = mpsc::channel(1024);
        let pipes = PipeClient::new(&"http://127.0.0.1:1".parse().unwrap(), tokio::sync::watch::Sender::new(None).subscribe()).unwrap();
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
            let value = reply(replies.recv().await.unwrap().into_bytes());
            if value["type"] != "git_ok" {
                failures.push(value.to_string());
            }
        }
        assert!(failures.is_empty(), "{failures:?}");
        host.close().await;
    })
    .await
    .unwrap();
}
