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
        let directory = tempfile::tempdir().unwrap();
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
        LocalInvocation {
            operation: "raw".into(),
            invocation_id: "invocation".into(),
            args: serde_json::to_value(RawCommand {
                context: id,
                root: "fixture".into(),
                argv,
                live: true,
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
        assert!(
            String::from_utf8(stdout)
                .unwrap()
                .starts_with("fixture: Test callback.")
        );
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
