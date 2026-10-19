use demi_command_protocol::{CommandCaller, CommandContext, CommandLocale, LocalInvocation};
use demi_runner_jobs::{
    commands::contexts::ExecutionContext,
    testing::{ContextGuard, Dispatch},
};
use demi_runner_process::{
    command_client::{RawCommand, Stdio, forward},
    pipes::PipeClient,
};
use demi_runner_protocol::wire::Inbound;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::watch,
};
use tokio_util::sync::CancellationToken;

struct NeverRead;
impl AsyncRead for NeverRead {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        panic!("input read by a command that should not consume it")
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    dispatch: Dispatch,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_backend("http://127.0.0.1:1").await
    }

    /// A fixture whose pipes go to the backend at `url`.
    async fn with_backend(url: &str) -> Self {
        Self::with_command(
            url,
            json!({
                "name": "fixture", "summary": "Test callback.", "kind": "rpc", "runningHint": "Working",
                "input": {"type": "object", "properties": {"body": {"type": "string"}}, "required": ["body"]}, "stdinField": "body"
            }),
        )
        .await
    }

    /// A fixture whose one command is `tree`, with its pipes at `url`.
    async fn with_command(url: &str, tree: Value) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let manifest = demi_runner_protocol::manifest::Manifest::build(
            [serde_json::from_value(tree).unwrap()],
            [],
        )
        .unwrap();
        let manifest = serde_json::to_value(manifest).unwrap();
        let token = "device-token".parse().unwrap();
        let pipes = PipeClient::new(
            &url.parse().unwrap(),
            watch::Sender::new(Some(token)).subscribe(),
        )
        .unwrap();
        let dispatch = Dispatch::new(directory.path(), manifest, pipes).await;
        Self {
            directory,
            dispatch,
        }
    }
    async fn context(&self) -> (Arc<ExecutionContext>, ContextGuard) {
        self.dispatch.context("job", command_context()).await
    }
    fn request(&self, id: String, argv: Vec<String>) -> LocalInvocation {
        self.request_with_input(id, argv, true)
    }
    /// A request whose process has a terminal for its input when `live`,
    /// and a finite stdin otherwise.
    fn request_with_input(&self, id: String, argv: Vec<String>, live: bool) -> LocalInvocation {
        LocalInvocation {
            operation: "raw".into(),
            invocation_id: "invocation".into(),
            args: serde_json::to_value(RawCommand {
                context: id,
                root: "fixture".into(),
                argv,
                live,
            })
            .unwrap(),
            cwd: self.directory.path().to_string_lossy().into_owned(),
            env: BTreeMap::new(),
        }
    }
    async fn message(&mut self) -> Value {
        let message = self.dispatch.outgoing.recv().await.unwrap();
        rmp_serde::from_slice(&message.into_bytes()).unwrap()
    }
    async fn close(self) {
        self.dispatch.close().await;
    }
}

#[tokio::test]
async fn help_never_reads_declared_stdin_or_calls_backend() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut fixture = Fixture::new().await;
        let (context, _lease) = fixture.context().await;
        let request = fixture.request(context.id.clone(), vec!["--help".into()]);
        let mut stdout = Vec::new();
        let result = forward(
            fixture.dispatch.server.endpoint(),
            &request,
            Stdio {
                stdin: NeverRead,
                stdout: &mut stdout,
                stderr: tokio::io::sink(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, 0);
        let help = String::from_utf8(stdout).unwrap();
        assert!(help.starts_with("Usage: fixture") && help.contains("\n\nTest callback."), "{help}");
        assert!(fixture.dispatch.outgoing.try_recv().is_err());
        fixture.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn callback_exit_clears_hint_and_revoked_context_cannot_dispatch() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut fixture = Fixture::new().await;
        let (context, lease) = fixture.context().await;
        let request = fixture.request(context.id.clone(), vec![]);
        let endpoint = fixture.dispatch.server.endpoint().to_owned();
        let request_copy = request.clone();
        let running = tokio::spawn(async move {
            forward(
                &endpoint,
                &request_copy,
                Stdio {
                    stdin: NeverRead,
                    stdout: tokio::io::sink(),
                    stderr: tokio::io::sink(),
                },
                CancellationToken::new(),
            )
            .await
            .unwrap()
        });
        assert_eq!(fixture.message().await["hint"], "Working");
        let call = fixture.message().await;
        assert_eq!(call["type"], "rpc_call");
        assert_eq!(call["args"]["body"], "");
        fixture
            .dispatch
            .deliver(Inbound::RpcExit {
                call_id: call["callId"].as_str().unwrap().into(),
                exit_code: 7,
            })
            .await;
        assert_eq!(running.await.unwrap().exit_code, 7);
        assert!(fixture.message().await["hint"].is_null());
        drop(lease);
        let result = forward(
            fixture.dispatch.server.endpoint(),
            &request,
            Stdio {
                stdin: NeverRead,
                stdout: tokio::io::sink(),
                stderr: tokio::io::sink(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_ne!(result.exit_code, 0);
        assert!(fixture.dispatch.outgoing.try_recv().is_err());
        fixture.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_sends_callback_cancel_and_clears_running_hint() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut fixture = Fixture::new().await;
        let (context, _lease) = fixture.context().await;
        let request = fixture.request(context.id.clone(), vec![]);
        let endpoint = fixture.dispatch.server.endpoint().to_owned();
        let cancel = CancellationToken::new();
        let stopped = cancel.clone();
        let running = tokio::spawn(async move {
            forward(
                &endpoint,
                &request,
                Stdio {
                    stdin: NeverRead,
                    stdout: tokio::io::sink(),
                    stderr: tokio::io::sink(),
                },
                stopped,
            )
            .await
        });
        assert_eq!(fixture.message().await["type"], "job_running_hint");
        assert_eq!(fixture.message().await["type"], "rpc_call");
        cancel.cancel();
        assert!(running.await.unwrap().is_err());
        let first = fixture.message().await;
        let second = fixture.message().await;
        assert_eq!(first["type"], "rpc_cancel");
        assert_eq!(second["type"], "job_running_hint");
        assert!(second["hint"].is_null());
        fixture.close().await;
    })
    .await
    .unwrap();
}

/// A backend whose one answer is a pipe's response that opens and breaks
/// off before its body ends, as a relayed stdout does when the job on
/// another device that it was handed to goes away.
async fn breaking_pipe() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        let head = "HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\n\
                    transfer-encoding: chunked\r\n\r\n";
        socket.write_all(head.as_bytes()).await.unwrap();
        // The connection ends before the body's last chunk.
    });
    format!("http://{address}/")
}

/// A relayed stdout that breaks after it opened ends the call with the
/// backend's exit, 1 and the cause on stderr, which the backend sends once
/// it sees the pipe fail (`commands.md` § Handle an rpc call). Before, the
/// call failed at once with the transport's own words, `error decoding
/// response body`, and the cause never showed.
#[tokio::test]
async fn a_relayed_stdout_that_breaks_ends_the_call_with_its_cause() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let backend = breaking_pipe().await;
        let mut fixture = Fixture::with_backend(&backend).await;
        let (context, _lease) = fixture.context().await;
        let request = fixture.request(context.id.clone(), vec![]);
        let endpoint = fixture.dispatch.server.endpoint().to_owned();
        let running = tokio::spawn(async move {
            let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
            let result = forward(
                &endpoint,
                &request,
                Stdio {
                    stdin: NeverRead,
                    stdout: &mut stdout,
                    stderr: &mut stderr,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();
            (result.exit_code, stdout, String::from_utf8(stderr).unwrap())
        });
        assert_eq!(fixture.message().await["hint"], "Working");
        let call = fixture.message().await;
        assert_eq!(call["type"], "rpc_call");
        assert_eq!(call["stdin"], false);
        let call_id = call["callId"].as_str().unwrap().to_owned();
        fixture
            .dispatch
            .deliver(Inbound::RpcPipes {
                call_id: call_id.clone(),
                stdin: None,
                stdout: demi_runner_protocol::wire::PipeRef {
                    id: "out".into(),
                    url: "/api/pipes/out".into(),
                },
            })
            .await;
        let done = fixture.message().await;
        assert_eq!((&done["type"], &done["ok"]), (&json!("pipe_done"), &json!(false)));
        let cause = "fixture: pipe failed: the job's device went away\n";
        fixture
            .dispatch
            .deliver(Inbound::RpcOutput {
                call_id: call_id.clone(),
                bytes: demi_runner_protocol::wire::WireBytes(cause.as_bytes().to_vec()),
            })
            .await;
        fixture
            .dispatch
            .deliver(Inbound::RpcExit {
                call_id,
                exit_code: 1,
            })
            .await;
        let (code, stdout, stderr) = running.await.unwrap();
        assert_eq!((code, stdout.as_slice(), stderr.as_str()), (1, &b""[..], cause));
        fixture.close().await;
    })
    .await
    .unwrap();
}

/// A backend whose pipe `out` answers a `GET` with `stdout` in data frames
/// and its end, and whose pipe `in` refuses a `PUT`, as the backend answers
/// the upload of a call's input once the call ended. `serving` hears once
/// the `GET` arrived.
async fn slow_call_pipes(stdout: Vec<u8>, serving: tokio::sync::oneshot::Sender<()>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut frames = bytes::BytesMut::new();
    demi_runner_protocol::wire::encode_data(&stdout, &mut frames);
    frames.extend_from_slice(&demi_runner_protocol::wire::end_frame(&Ok(())));
    let frames = frames.freeze();
    tokio::spawn(async move {
        let mut serving = Some(serving);
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                head.push(byte[0]);
            }
            if head.starts_with(b"GET /api/pipes/out ") {
                let frames = frames.clone();
                let serving = serving.take().unwrap();
                tokio::spawn(async move {
                    let answer = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\n\
                         content-length: {}\r\n\r\n",
                        frames.len()
                    );
                    socket.write_all(answer.as_bytes()).await.unwrap();
                    serving.send(()).unwrap();
                    // A call that ends before it read the body closes the
                    // connection; the test's assertions say what arrived.
                    let _ = socket.write_all(&frames).await;
                });
            } else {
                assert!(head.starts_with(b"PUT /api/pipes/in "));
                let refusal = "pipe failed: rpc call ended";
                let answer = format!(
                    "HTTP/1.1 409 Conflict\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{refusal}",
                    refusal.len()
                );
                socket.write_all(answer.as_bytes()).await.unwrap();
            }
        }
    });
    format!("http://{address}/")
}

/// A call's stdout reaches a process that reads it slowly whole, and the
/// call exits with the backend's code, after its unused input's upload was
/// refused as the backend refuses it once the call ended (`commands.md`
/// § Handle an rpc call). Before, the refused upload gave the rest of the
/// call 15 seconds: `demi image generate … > fox.png` to a slow disk was
/// cut at a multiple of 64 KiB with `rpc call ended`. The clock is paused
/// while the process reads nothing, so those seconds pass at once. Costs
/// about a second: 8 MiB through the local pipes.
#[tokio::test]
async fn a_calls_stdout_reaches_a_slow_process_whole_once_its_input_ended() {
    use tokio::io::AsyncReadExt;
    tokio::time::timeout(Duration::from_secs(60), async {
        let image: Vec<u8> = (0..8 << 20).map(|index: u32| (index % 251) as u8).collect();
        let (serving, served) = tokio::sync::oneshot::channel();
        let backend = slow_call_pipes(image.clone(), serving).await;
        let mut fixture = Fixture::with_command(
            &backend,
            json!({"name": "fixture", "summary": "Test callback.", "kind": "rpc"}),
        )
        .await;
        let (context, _lease) = fixture.context().await;
        let request = fixture.request_with_input(context.id.clone(), vec![], false);
        let endpoint = fixture.dispatch.server.endpoint().to_owned();
        let (mut process, stdout) = tokio::io::duplex(64 * 1024);
        let running = tokio::spawn(async move {
            let mut stderr = Vec::new();
            let result = forward(
                &endpoint,
                &request,
                Stdio {
                    stdin: tokio::io::empty(),
                    stdout,
                    stderr: &mut stderr,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();
            (result.exit_code, String::from_utf8(stderr).unwrap())
        });
        let call = fixture.message().await;
        assert_eq!(call["type"], "rpc_call");
        assert_eq!(call["stdin"], true);
        let call_id = call["callId"].as_str().unwrap().to_owned();
        assert_eq!(fixture.message().await["type"], "rpc_stdin_end");
        fixture
            .dispatch
            .deliver(Inbound::RpcPipes {
                call_id: call_id.clone(),
                stdin: Some(demi_runner_protocol::wire::PipeRef {
                    id: "in".into(),
                    url: "/api/pipes/in".into(),
                }),
                stdout: demi_runner_protocol::wire::PipeRef {
                    id: "out".into(),
                    url: "/api/pipes/out".into(),
                },
            })
            .await;
        let refused = fixture.message().await;
        assert_eq!(
            (&refused["type"], &refused["pipeId"], &refused["ok"]),
            (&json!("pipe_done"), &json!("in"), &json!(false))
        );
        fixture
            .dispatch
            .deliver(Inbound::RpcExit {
                call_id,
                exit_code: 0,
            })
            .await;
        served.await.unwrap();
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(20)).await;
        tokio::time::resume();
        let mut written = Vec::new();
        process.read_to_end(&mut written).await.unwrap();
        let (code, stderr) = running.await.unwrap();
        assert_eq!((code, stderr.as_str()), (0, ""));
        assert!(written == image, "{} of {} bytes arrived", written.len(), image.len());
        let done = fixture.message().await;
        assert_eq!(
            (&done["type"], &done["pipeId"], &done["ok"]),
            (&json!("pipe_done"), &json!("out"), &json!(true))
        );
        fixture.close().await;
    })
    .await
    .unwrap();
}

fn command_context() -> CommandContext {
    CommandContext {
        color_scheme: demi_command_protocol::ColorScheme::Light,
        conversation: "conversation".into(),
        caller: CommandCaller::agent(1),
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
    }
}

/// What a native command's run says on its standard error while its
/// program's install waits on a backend that never says where the program
/// is: with `--json`, nothing, since its caller reads its standard error as
/// one JSON document; without, that it installs, once the install has taken
/// two seconds (`native-runtime.md` § Install artifacts). The run with
/// `--json` starts first, so its two seconds have passed when the other
/// tells. Cost: in process, about two seconds.
#[tokio::test]
async fn a_command_run_with_json_reports_no_install() {
    use demi_command_protocol::{PackageArtifact, PackageDescriptor, host_target, testing::FIXTURE_OPERATIONS};
    use tokio::io::AsyncBufReadExt as _;
    tokio::time::timeout(Duration::from_secs(60), async {
        let descriptor = PackageDescriptor {
            id: "demi.fixture".into(),
            version: "1.0.0".into(),
            protocol_version: 1,
            operations: FIXTURE_OPERATIONS.map(String::from).to_vec(),
            targets: BTreeMap::from([(
                host_target().into(),
                PackageArtifact {
                    sha256: "a".repeat(64),
                    size: 1 << 20,
                },
            )]),
        };
        let tree = json!({
            "name": "fixture", "summary": "Fixture.", "kind": "native",
            "binding": {"package": "demi.fixture", "operation": "echo"},
            "output": {"json": {"type": "object"}}
        });
        let directory = tempfile::tempdir().unwrap();
        let manifest = demi_runner_protocol::manifest::Manifest::build(
            [serde_json::from_value(tree).unwrap()],
            [descriptor],
        )
        .unwrap();
        let pipes = PipeClient::new(
            &"http://127.0.0.1:1".parse().unwrap(),
            watch::Sender::new(None).subscribe(),
        )
        .unwrap();
        let mut dispatch =
            Dispatch::new(directory.path(), serde_json::to_value(manifest).unwrap(), pipes).await;
        let (context, _guard) = dispatch.context("job", command_context()).await;
        let cancel = CancellationToken::new();
        // Runs the command with `argv`, its standard error going to `stderr`.
        let run = |argv: Vec<String>, stderr: tokio::io::DuplexStream| {
            let request = LocalInvocation {
                operation: "raw".into(),
                invocation_id: "invocation".into(),
                args: serde_json::to_value(RawCommand {
                    context: context.id.clone(),
                    root: "fixture".into(),
                    argv,
                    live: false,
                })
                .unwrap(),
                cwd: directory.path().to_string_lossy().into_owned(),
                env: BTreeMap::new(),
            };
            let endpoint = dispatch.server.endpoint().to_owned();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let stdio = Stdio {
                    stdin: tokio::io::empty(),
                    stdout: tokio::io::sink(),
                    stderr,
                };
                // Cancelled: how it ended says nothing here.
                let _ended = forward(&endpoint, &request, stdio, cancel).await;
            })
        };
        let (json_stderr, mut json_read) = tokio::io::duplex(1 << 16);
        let quiet = run(vec!["--json".into()], json_stderr);
        // Its install asks the backend where the program is, which never
        // answers; the second run joins that install.
        loop {
            let message = dispatch.outgoing.recv().await.unwrap();
            let value: Value = rmp_serde::from_slice(&message.into_bytes()).unwrap();
            if value["type"] == "artifact_resolve" {
                break;
            }
        }
        let (stderr, told) = tokio::io::duplex(1 << 16);
        let telling = run(Vec::new(), stderr);
        let mut told = tokio::io::BufReader::new(told).lines();
        let line = told.next_line().await.unwrap().unwrap();
        assert_eq!(line, "Installing demi.fixture 1.0.0 (1.0 MB)…");
        cancel.cancel();
        quiet.await.unwrap();
        telling.await.unwrap();
        let mut said = String::new();
        tokio::io::AsyncReadExt::read_to_string(&mut json_read, &mut said).await.unwrap();
        assert!(!said.contains("Installing"), "{said}");
        dispatch.close().await;
    })
    .await
    .unwrap();
}
