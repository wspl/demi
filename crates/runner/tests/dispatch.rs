use demi_command_service::protocol::Invocation;
use demi_runner::{
    commands::artifacts::Artifacts,
    commands::command_client::{RawCommand, Stdio, forward},
    commands::contexts::Contexts,
    commands::dispatch::Dispatcher,
    commands::local::Server,
    commands::native::{self, Services},
    commands::rpc::Calls,
    management::Management,
    pipes::PipeClient,
};
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
    sync::{RwLock, mpsc},
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
    _directory: tempfile::TempDir,
    contexts: Contexts,
    services: Arc<Services>,
    calls: Arc<Calls>,
    server: Server,
    dispatcher: Arc<Dispatcher>,
    outgoing: mpsc::Receiver<demi_runner::connection::wire::Outbound>,
    hash: String,
}

impl Fixture {
    async fn new() -> Self {
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
            cwd,
            BTreeMap::new(),
        )
        .await
        .unwrap();
        let resolver = Artifacts::new(contexts.clone(), native::target().into());
        let calls =
            Calls::new(PipeClient::new("http://127.0.0.1:1", Arc::new(RwLock::new(None))).unwrap());
        let (output, outgoing) = mpsc::channel(32);
        calls.attach(output, CancellationToken::new());
        let management = Management::new("a".repeat(32), "test".into(), CancellationToken::new());
        let dispatcher = Arc::new(Dispatcher {
            contexts: contexts.clone(),
            services: services.clone(),
            resolver,
            calls: calls.clone(),
            management,
        });
        let server = Server::start(dispatcher.clone()).await.unwrap();
        Self {
            _directory: directory,
            contexts,
            services,
            calls,
            server,
            dispatcher,
            outgoing,
            hash,
        }
    }
    async fn context(
        &self,
    ) -> (
        Arc<demi_runner::commands::contexts::ExecutionContext>,
        demi_runner::commands::contexts::Lease,
    ) {
        self.contexts
            .create(
                "job".into(),
                &self.hash,
                &BTreeMap::from([
                    ("DEMI_SESSION_ID".into(), "session".into()),
                    ("DEMI_SHELL_ID".into(), "shell".into()),
                ]),
            )
            .await
            .unwrap()
    }
    fn request(&self, id: String, argv: Vec<String>) -> Invocation {
        Invocation {
            operation: "raw".into(),
            invocation_id: "invocation".into(),
            args: serde_json::to_value(RawCommand {
                context: id,
                root: "fixture".into(),
                argv,
                live: true,
            })
            .unwrap(),
            cwd: self._directory.path().to_string_lossy().into_owned(),
            env: BTreeMap::new(),
        }
    }
    async fn message(&mut self) -> Value {
        let message = self.outgoing.recv().await.unwrap();
        rmp_serde::from_slice(&message.into_bytes()).unwrap()
    }
    async fn close(self) {
        self.contexts.close();
        self.calls.detach();
        self.server.close().await.unwrap();
        self.services.close().await;
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
            fixture.server.endpoint(),
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
        assert!(fixture.outgoing.try_recv().is_err());
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
        let endpoint = fixture.server.endpoint().to_owned();
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
            .calls
            .reply(&demi_runner::connection::wire::Inbound::RpcExit {
                call_id: call["callId"].as_str().unwrap().into(),
                exit_code: 7.0,
            });
        assert_eq!(running.await.unwrap().exit_code, 7);
        assert!(fixture.message().await["hint"].is_null());
        drop(lease);
        let result = forward(
            fixture.server.endpoint(),
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
        assert!(fixture.outgoing.try_recv().is_err());
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
        let endpoint = fixture.server.endpoint().to_owned();
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
                dispatcher: fixture.dispatcher.clone(),
                execution: context,
            }),
        );
        // Neither PATH aliases nor a local endpoint are supplied to the shell.
        let mut job = Job::start(
            "printf body | fixture".into(),
            fixture._directory.path().into(),
            BTreeMap::new(),
            true,
            scope,
        )
        .unwrap();
        assert_eq!(fixture.message().await["hint"], "Working");
        let call = fixture.message().await;
        assert_eq!(call["type"], "rpc_call");
        assert_eq!(call["args"]["body"], "body");
        fixture
            .calls
            .reply(&demi_runner::connection::wire::Inbound::RpcExit {
                call_id: call["callId"].as_str().unwrap().into(),
                exit_code: 7.0,
            });
        let (exit, _) = job.wait().await;
        assert_eq!(exit.code, Some(7), "{:?}", exit.error);
        assert!(fixture.message().await["hint"].is_null());
        fixture.close().await;
    })
    .await
    .unwrap();
}
