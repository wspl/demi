use demi_command_service::protocol::{
    CommandCaller, CommandContext, CommandLocale, LocalInvocation,
};
use demi_runner::{
    commands::command_client::{RawCommand, Stdio, forward},
    commands::contexts::ExecutionContext,
    pipes::PipeClient,
    testing::{ContextGuard, Dispatch},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{io::{AsyncRead, ReadBuf}, sync::watch};
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
        let directory = tempfile::tempdir().unwrap();
        let body = json!({"roots": {"fixture": {"tree": {
            "name": "fixture", "summary": "Test callback.", "kind": "rpc", "runningHint": "Working",
            "input": {"type": "object", "properties": {"body": {"type": "string"}}, "required": ["body"]}, "stdinField": "body"
        }}}, "packages": {}});
        let hash = demi_command_service::protocol::canonical_digest(&body).unwrap();
        let mut manifest = body;
        manifest["hash"] = hash.into();
        let pipes = PipeClient::new("http://127.0.0.1:1", watch::Sender::new(None).subscribe()).unwrap();
        let dispatch = Dispatch::new(directory.path(), manifest, pipes).await;
        Self { directory, dispatch }
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
    tokio::time::timeout(Duration::from_secs(10), async {
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
    tokio::time::timeout(Duration::from_secs(10), async {
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
            .deliver(demi_runner::connection::wire::Inbound::RpcExit {
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
    tokio::time::timeout(Duration::from_secs(10), async {
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

#[tokio::test]
async fn declared_shell_builtin_dispatches_without_a_local_endpoint() {
    use demi_runner::shell::{
        job::Job,
        scope::{CommandContext, Scope},
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut fixture = Fixture::new().await;
        let (context, _lease) = fixture.context().await;
        let scope = Scope::new(
            CancellationToken::new(),
            Some(CommandContext {
                dispatcher: fixture.dispatch.dispatcher.clone(),
                execution: context,
            }),
        );
        // Neither PATH aliases nor a local endpoint are supplied to the shell.
        let mut job = Job::start(
            "printf body | fixture".into(),
            fixture.directory.path().into(),
            BTreeMap::new(),
            true,
            scope, &demi_runner::shell::ShellRuntime::current(),
        )
            .await
        .unwrap();
        assert_eq!(fixture.message().await["hint"], "Working");
        let call = fixture.message().await;
        assert_eq!(call["type"], "rpc_call");
        assert_eq!(call["args"]["body"], "body");
        fixture
            .dispatch
            .deliver(demi_runner::connection::wire::Inbound::RpcExit {
                call_id: call["callId"].as_str().unwrap().into(),
                exit_code: 7,
            })
            .await;
        let (exit, _) = job.wait().await;
        assert_eq!(exit.code, Some(7), "{:?}", exit.error);
        assert!(fixture.message().await["hint"].is_null());
        fixture.close().await;
    })
    .await
    .unwrap();
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
