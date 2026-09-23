#![cfg(all(unix, feature = "test-fixtures"))]
//! With no open file left, whatever the runner needs one for waits until one
//! closes, then finishes (`runner.md` § Load). One test in its own binary: it
//! lowers the process's open-file limit and holds every remaining descriptor.

use demi_command_service::protocol::{
    CommandCaller, CommandContext, CommandLocale, LocalInvocation, PackageArtifact,
    PackageDescriptor,
};
use demi_runner::{
    commands::artifacts::Artifacts,
    commands::command_client::{RawCommand, Stdio, forward},
    commands::contexts::Contexts,
    commands::dispatch::Dispatcher,
    commands::local::Server,
    commands::rpc::Calls,
    connection::wire::{Inbound, PipeRef},
    host::HostServer,
    management::Management,
    pipes::PipeClient,
    process::{ChildProcess, SpawnOptions},
    services::{ArtifactResolver, ArtifactSource, RuntimeError, ServiceRegistry, target},
    shell::{job::Job, scope::Scope},
};
use futures_util::{StreamExt, future::BoxFuture};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{RwLock, mpsc},
};
use tokio_util::sync::CancellationToken;

/// Every descriptor the process has left, held open.
struct Hog(Vec<std::fs::File>);

impl Hog {
    fn fill() -> Self {
        let mut files = Vec::new();
        loop {
            match std::fs::File::open("/dev/null") {
                Ok(file) => files.push(file),
                Err(error) if error.raw_os_error() == Some(libc::EMFILE) => return Self(files),
                Err(error) => panic!("filling descriptors: {error}"),
            }
        }
    }
}

fn set_soft_limit(value: u64) {
    unsafe {
        let mut limit = std::mem::zeroed::<libc::rlimit>();
        assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit), 0);
        limit.rlim_cur = value as libc::rlim_t;
        assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &limit), 0);
    }
}

/// Starts `operation` with no descriptor left and requires it to wait rather
/// than finish or fail; then frees 128 descriptors and requires it to finish.
/// Starting a native service copies and checks a 15 MB executable, and macOS
/// can take seconds to run one it has not run before, so the wait after
/// freeing is generous.
async fn starved<F>(name: &str, operation: F)
where
    F: Future<Output = Result<(), String>> + Send + 'static,
{
    let mut hog = Hog::fill();
    // Take any descriptor an earlier step closes late, too.
    tokio::time::sleep(Duration::from_millis(100)).await;
    hog.0.extend(Hog::fill().0);
    let task = tokio::spawn(operation);
    tokio::time::sleep(Duration::from_millis(300)).await;
    if task.is_finished() {
        let outcome = task.await.unwrap();
        panic!("{name} did not wait with no open file left: {outcome:?}");
    }
    let keep = hog.0.len().saturating_sub(128);
    hog.0.truncate(keep);
    let outcome = tokio::time::timeout(Duration::from_secs(30), task)
        .await
        .unwrap_or_else(|_| panic!("{name} still waits after files were freed"))
        .unwrap();
    if let Err(error) = outcome {
        panic!("{name} failed: {error}");
    }
}

/// A backend that answers pipe downloads with one chunk and holds them open,
/// and accepts pipe uploads. It retries its own accept when out of files.
async fn backend() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(5)).await;
                continue;
            };
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(count) => request.extend_from_slice(&buffer[..count]),
                    }
                }
                if request.starts_with(b"PUT") {
                    while !request.windows(5).any(|window| window == b"0\r\n\r\n") {
                        match socket.read(&mut buffer).await {
                            Ok(0) | Err(_) => return,
                            Ok(count) => request.extend_from_slice(&buffer[..count]),
                        }
                    }
                    let _ = socket
                        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                        .await;
                } else {
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n2\r\nok\r\n",
                        )
                        .await;
                }
                let _ = socket.read(&mut buffer).await;
            });
        }
    });
    port
}

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

fn reply(bytes: Vec<u8>) -> serde_json::Value {
    rmp_serde::from_slice(&bytes).unwrap()
}

/// The next reply whose `key` is `value`; replies for other requests, such as
/// a finished pipe's report, are skipped.
async fn reply_for(
    replies: &tokio::sync::Mutex<mpsc::Receiver<demi_runner::connection::wire::Frame>>,
    key: &str,
    value: &str,
) -> serde_json::Value {
    loop {
        let message = reply(replies.lock().await.recv().await.unwrap().into_bytes());
        if message[key] == value {
            return message;
        }
    }
}

fn repository(path: &std::path::Path) {
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(path)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .unwrap();
        assert!(status.success());
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(path.join("a.txt"), "1\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "first"]);
    std::fs::write(path.join("a.txt"), "2\n").unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn running_out_of_open_files_waits_instead_of_failing() {
    set_soft_limit(1024);
    let root = tempfile::tempdir().unwrap();
    let root_path = std::fs::canonicalize(root.path()).unwrap();

    // Pipes.
    let port = backend().await;
    let pipes = PipeClient::new(
        &format!("http://127.0.0.1:{port}"),
        Arc::new(RwLock::new(Some("token".into()))),
    )
    .unwrap();
    {
        let pipes = pipes.clone();
        starved("pipe download", async move {
            let mut stream = pipes
                .get("/pipe/download", CancellationToken::new())
                .await
                .map_err(|error| error.to_string())?;
            match stream.next().await {
                Some(Ok(_)) => Ok(()),
                other => Err(format!("{other:?}")),
            }
        })
        .await;
    }
    {
        let pipes = pipes.clone();
        starved("pipe upload", async move {
            let body = futures_util::stream::iter([Ok(bytes::Bytes::from_static(b"hello"))]);
            pipes
                .put("/pipe/upload", body, &CancellationToken::new())
                .await
                .map_err(|error| error.to_string())
        })
        .await;
    }

    // Host requests.
    let (output, replies) = mpsc::channel(64);
    let replies = Arc::new(tokio::sync::Mutex::new(replies));
    let host = Arc::new(HostServer::new(
        output,
        None,
        root_path.join("logs"),
        root_path.clone(),
        BTreeMap::new(),
        pipes.clone(),
    ));
    {
        let host = host.clone();
        let replies = replies.clone();
        starved("filesystem request", async move {
            host.handle_filesystem(Inbound::FsReaddir {
                id: "r".into(),
                path: ".".into(),
                cwd: None,
                with_file_types: Some(true),
            })
            .map_err(|error| error.to_string())?;
            let value = reply_for(&replies, "id", "r").await;
            (value["type"] == "fs_ok")
                .then_some(())
                .ok_or(value.to_string())
        })
        .await;
    }
    std::fs::write(root_path.join("transfer.txt"), "contents").unwrap();
    {
        let host = host.clone();
        let replies = replies.clone();
        starved("file transfer", async move {
            host.handle_filesystem(Inbound::FsReadFile {
                id: "t".into(),
                path: "transfer.txt".into(),
                cwd: None,
                offset: None,
                length: None,
                output: PipeRef {
                    id: "transfer".into(),
                    url: "/pipe/transfer".into(),
                },
            })
            .map_err(|error| error.to_string())?;
            let value = reply_for(&replies, "id", "t").await;
            (value["type"] == "fs_ok")
                .then_some(())
                .ok_or(value.to_string())
        })
        .await;
    }
    let repo = root_path.join("repo");
    std::fs::create_dir(&repo).unwrap();
    repository(&repo);
    {
        let host = host.clone();
        let replies = replies.clone();
        let repo = repo.to_string_lossy().into_owned();
        starved("working-tree request", async move {
            host.handle_git(Inbound::GitChanges {
                id: "g".into(),
                root: repo,
            })
            .map_err(|error| error.to_string())?;
            let value = reply_for(&replies, "id", "g").await;
            // Out of open files must never read as "not a repository", nor
            // hide a change.
            let result = &value["result"];
            (value["type"] == "git_ok"
                && result["repository"] == true
                && result["files"][0]["path"] == "a.txt"
                && result["files"][0]["kind"] == "modified")
                .then_some(())
                .ok_or(value.to_string())
        })
        .await;
    }
    let service = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let service_port = service.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            match service.accept().await {
                Ok((socket, _)) => held.push(socket),
                Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
    });
    {
        let host = host.clone();
        let replies = replies.clone();
        starved("network stream", async move {
            host.handle_net(Inbound::NetOpen {
                stream_id: "n".into(),
                host: "127.0.0.1".into(),
                port: service_port.into(),
                input: PipeRef {
                    id: "in".into(),
                    url: "/pipe/net-in".into(),
                },
                output: PipeRef {
                    id: "out".into(),
                    url: "/pipe/net-out".into(),
                },
            })
            .map_err(|error| error.to_string())?;
            let value = reply_for(&replies, "streamId", "n").await;
            (value["type"] == "net_opened")
                .then_some(())
                .ok_or(value.to_string())
        })
        .await;
    }

    // Processes and shell jobs.
    {
        let cwd = root_path.clone();
        starved("process", async move {
            let mut child = ChildProcess::spawn(SpawnOptions {
                command: "/bin/echo".into(),
                args: vec!["hello".into()],
                cwd,
                env: BTreeMap::new(),
                process_group: true,
            })
            .await
            .map_err(|failure| failure.message)?;
            let exit = child.wait().await;
            (exit.code == Some(0))
                .then_some(())
                .ok_or(format!("{exit:?}"))
        })
        .await;
    }
    {
        let cwd = root_path.clone();
        starved("shell job with a pipeline and a redirection", async move {
            let mut job = Job::start(
                "echo one | cat > piped.txt".into(),
                cwd.clone(),
                BTreeMap::new(),
                false,
                Scope::new(CancellationToken::new(), None),
            )
            .map_err(|error| error.to_string())?;
            let (exit, _) = job.wait().await;
            let written = std::fs::read_to_string(cwd.join("piped.txt")).unwrap_or_default();
            (exit.code == Some(0) && written == "one\n")
                .then_some(())
                .ok_or(format!("{exit:?} {written:?}"))
        })
        .await;
    }

    // A resident native service.
    {
        let cache = root_path.join("native");
        let services = ServiceRegistry::new(cache.join("cache"), root_path.clone(), BTreeMap::new())
            .await
            .unwrap();
        let bytes = std::fs::read(env!("CARGO_BIN_EXE_demi-native-fixture")).unwrap();
        let path = cache.join("fixture");
        std::fs::write(&path, &bytes).unwrap();
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
        let started = services.handle();
        starved("native service start", async move {
            let resident = started
                .acquire(
                    &descriptor,
                    Arc::new(Local(path)),
                    &CancellationToken::new(),
                )
                .await
                .map_err(|error| error.to_string())?;
            resident
                .client()
                .info()
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
        .await;
        services.close().await;
    }

    // Local command connections through the runner's endpoint.
    let services = ServiceRegistry::new(root_path.join("artifacts"), root_path.clone(), BTreeMap::new())
        .await
        .unwrap();
    let contexts = Contexts::new(
        root_path.join("manifests"),
        std::env::current_exe().unwrap(),
        services.handle(),
    )
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
    let calls = Calls::new(pipes.clone());
    let (output, mut outgoing) = mpsc::channel(32);
    calls.attach(output, CancellationToken::new());
    let reached = Arc::new(AtomicUsize::new(0));
    let counted = reached.clone();
    tokio::spawn(async move {
        while let Some(message) = outgoing.recv().await {
            let value = reply(message.into_bytes());
            if value["type"] == "rpc_call" {
                counted.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    let dispatcher = Arc::new(Dispatcher {
        contexts: contexts.clone(),
        services: services.handle(),
        resolver: Artifacts::new(contexts.clone(), target().into()),
        calls: calls.clone(),
        management: Management::new("a".repeat(32), "test".into(), CancellationToken::new()),
    });
    let server = Server::start(dispatcher).await.unwrap();
    let (context, _lease) = contexts
        .create("job".into(), &hash, command_context())
        .await
        .unwrap();
    let request = LocalInvocation {
        operation: "raw".into(),
        invocation_id: "invocation".into(),
        args: serde_json::to_value(RawCommand {
            context: context.id.clone(),
            root: "fixture".into(),
            argv: vec![],
            live: true,
        })
        .unwrap(),
        cwd: root_path.to_string_lossy().into_owned(),
        env: BTreeMap::new(),
    };
    let cancel = CancellationToken::new();
    {
        let endpoint = server.endpoint().to_owned();
        let request = request.clone();
        let cancel = cancel.clone();
        let reached = reached.clone();
        starved("local command connection", async move {
            tokio::spawn(async move {
                let _ = forward(
                    &endpoint,
                    &request,
                    Stdio {
                        stdin: tokio::io::empty(),
                        stdout: tokio::io::sink(),
                        stderr: tokio::io::sink(),
                    },
                    cancel,
                )
                .await;
            });
            while reached.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok(())
        })
        .await;
    }
    cancel.cancel();
    contexts.close();
    calls.detach();
    server.close().await.unwrap();
    services.close().await;
}

fn command_context() -> CommandContext {
    CommandContext {
        conversation: "conversation".into(),
        caller: CommandCaller::agent("session"),
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
    }
}
