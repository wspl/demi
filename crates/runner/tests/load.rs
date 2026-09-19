#![cfg(feature = "test-fixtures")]
//! No request the runner serves is refused for how many others are in flight
//! (`runner.md` § Load): native calls and commands the backend implements are
//! not counted, and bounded Host work waits for a slot.

use demi_command_service::{
    Client,
    protocol::{Invocation, PackageArtifact, PackageDescriptor, Record},
};
use demi_runner::{
    commands::artifacts::Artifacts,
    commands::cache::{ArtifactResolver, ArtifactSource, RuntimeError},
    commands::command_client::{RawCommand, Stdio, forward},
    commands::contexts::Contexts,
    commands::dispatch::Dispatcher,
    commands::local::Server,
    commands::native::{self, Services},
    commands::rpc::Calls,
    connection::wire::Inbound,
    host::HostServer,
    management::Management,
    pipes::PipeClient,
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
use tokio::sync::{RwLock, mpsc};
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

fn invocation(operation: &str, conversation: &str) -> Invocation {
    Invocation {
        conversation: conversation.into(),
        caller: "node".into(),
        operation: operation.into(),
        invocation_id: operation.into(),
        args: json!({}),
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        env: BTreeMap::new(),
        edits: None,
        json: None,
    }
}

async fn resident(root: &Path) -> (Arc<Services>, Arc<Client>) {
    let services = Services::new(
        root.join("cache"),
        native::target().into(),
        root.into(),
        BTreeMap::new(),
    )
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
        operations: ["where", "echo", "first", "spin", "result", "retain"]
            .map(String::from)
            .to_vec(),
        targets: BTreeMap::from([(
            native::target().into(),
            PackageArtifact {
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                size: bytes.len() as u64,
            },
        )]),
    };
    let client = services
        .acquire(
            &descriptor,
            Arc::new(Local(path)),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    (services, client)
}

/// Native calls held open, then a cancel and a conversation release at once:
/// the release is admitted, and every other call keeps running.
#[tokio::test(flavor = "multi_thread")]
async fn native_calls_are_never_turned_away() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let (services, client) = resident(root.path()).await;
        let mut held = Vec::new();
        for _ in 0..128 {
            held.push(client.invoke(&invocation("echo", "running")).await.unwrap());
        }
        for attempt in 0..40 {
            let (mut input, _output) = held.remove(0);
            input.cancel();
            services
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
        let contexts = Contexts::new(cwd.join("manifests"), std::env::current_exe().unwrap())
            .await
            .unwrap();
        let body = json!({"roots": {"fixture": {"tree": {
            "name": "fixture", "summary": "Test callback.", "kind": "rpc", "runningHint": "Working",
            "input": {"type": "object", "properties": {"body": {"type": "string"}}, "required": ["body"]}, "stdinField": "body"
        }}}, "packages": {}});
        let hash = demi_command_service::protocol::canonical_digest(&body).unwrap();
        let mut manifest = body;
        manifest["hash"] = hash.clone().into();
        contexts.install(manifest).await.unwrap();
        let services = Services::new(
            cwd.join("artifacts"),
            native::target().into(),
            cwd.clone(),
            BTreeMap::new(),
        )
        .await
        .unwrap();
        let resolver = Artifacts::new(contexts.clone(), native::target().into());
        let calls =
            Calls::new(PipeClient::new("http://127.0.0.1:1", Arc::new(RwLock::new(None))).unwrap());
        let (output, mut outgoing) = mpsc::channel(32);
        calls.attach(output, CancellationToken::new());
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
        let management = Management::new("a".repeat(32), "test".into(), CancellationToken::new());
        let dispatcher = Arc::new(Dispatcher {
            contexts: contexts.clone(),
            services: services.clone(),
            resolver,
            calls: calls.clone(),
            management,
        });
        let server = Server::start(dispatcher).await.unwrap();
        let (context, _lease) = contexts
            .create(
                "job".into(),
                &hash,
                "conversation".into(),
                "session".into(),
                &BTreeMap::from([
                    ("DEMI_SESSION_ID".into(), "session".into()),
                    ("DEMI_SHELL_ID".into(), "shell".into()),
                ]),
            )
            .await
            .unwrap();
        let request = Invocation {
            args: serde_json::to_value(RawCommand {
                context: context.id.clone(),
                root: "fixture".into(),
                argv: vec![],
                live: true,
            })
            .unwrap(),
            operation: "raw".into(),
            cwd: cwd.to_string_lossy().into_owned(),
            ..invocation("raw", "runner-local")
        };
        let cancel = CancellationToken::new();
        let mut running = tokio::task::JoinSet::new();
        for _ in 0..200 {
            let endpoint = server.endpoint().to_owned();
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
        contexts.close();
        calls.detach();
        server.close().await.unwrap();
        services.close().await;
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
        let pipes = PipeClient::new("http://127.0.0.1:1", Arc::new(RwLock::new(None))).unwrap();
        let host = HostServer::new(
            output,
            None,
            root.path().join("logs"),
            root.path().into(),
            BTreeMap::new(),
            pipes,
        );
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
        let pipes = PipeClient::new("http://127.0.0.1:1", Arc::new(RwLock::new(None))).unwrap();
        let logs = tempfile::tempdir().unwrap();
        let host = HostServer::new(
            output,
            None,
            logs.path().join("logs"),
            logs.path().into(),
            BTreeMap::new(),
            pipes,
        );
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
