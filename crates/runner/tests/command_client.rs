use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use demi_command_service::{
    Handler, InvocationContext, ServiceError,
    protocol::{Completion, LocalInvocation},
};
use demi_runner::{
    commands::command_client::{Stdio, forward},
    commands::local::Server,
};
use futures_util::FutureExt;
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::sync::CancellationToken;

struct Commands;

impl Handler for Commands {
    type Metadata = LocalInvocation;

    fn operations(&self) -> Vec<String> {
        vec![
            "no-input".into(),
            "pending-input".into(),
            "blocked-output".into(),
        ]
    }
    fn invoke(
        &self,
        mut context: InvocationContext<LocalInvocation>,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Box::pin(async move {
            if context.request.operation == "pending-input" {
                assert!(context.input.next().now_or_never().is_none());
            }
            if context.request.operation == "blocked-output" {
                loop {
                    context.output.stdout(Bytes::from(vec![0; 65536])).await?;
                }
            }
            context
                .output
                .stdout(Bytes::from_static(b"done\0\xff"))
                .await?;
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }
}

struct NeverRead;
impl AsyncRead for NeverRead {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        panic!("stdin was consumed without input demand");
    }
}

fn invocation(operation: &str) -> LocalInvocation {
    LocalInvocation {
        operation: operation.into(),
        invocation_id: operation.into(),
        args: serde_json::json!({}),
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        env: BTreeMap::new(),
    }
}

#[tokio::test]
async fn command_that_does_not_read_stdin_never_polls_the_source() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let server = Server::start(Arc::new(Commands)).await.unwrap();
        let mut stdout = Vec::new();
        let completion = forward(
            server.endpoint(),
            &invocation("no-input"),
            Stdio {
                stdin: NeverRead,
                stdout: &mut stdout,
                stderr: tokio::io::sink(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(completion.exit_code, 0);
        assert_eq!(stdout, b"done\0\xff");
        server.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn pending_terminal_input_does_not_block_output_or_completion() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let server = Server::start(Arc::new(Commands)).await.unwrap();
        let (_terminal, stdin) = tokio::io::duplex(1);
        let mut stdout = Vec::new();
        let completion = forward(
            server.endpoint(),
            &invocation("pending-input"),
            Stdio {
                stdin,
                stdout: &mut stdout,
                stderr: tokio::io::sink(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(completion.exit_code, 0);
        assert_eq!(stdout, b"done\0\xff");
        server.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_interrupts_blocked_output() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let server = Server::start(Arc::new(Commands)).await.unwrap();
        let (stdout, mut receiver) = tokio::io::duplex(1);
        let endpoint = server.endpoint().to_owned();
        let cancel = CancellationToken::new();
        let caller = cancel.clone();
        let task = tokio::spawn(async move {
            forward(
                &endpoint,
                &invocation("blocked-output"),
                Stdio {
                    stdin: NeverRead,
                    stdout,
                    stderr: tokio::io::sink(),
                },
                caller,
            )
            .await
        });
        use tokio::io::AsyncReadExt;
        receiver.read_u8().await.unwrap();
        cancel.cancel();
        assert_eq!(
            task.await.unwrap().unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
        server.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn connection_loss_interrupts_a_caller_blocked_on_stdout() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let server = Server::start(Arc::new(Commands)).await.unwrap();
        let (stdout, mut receiver) = tokio::io::duplex(1);
        let endpoint = server.endpoint().to_owned();
        let task = tokio::spawn(async move {
            forward(
                &endpoint,
                &invocation("blocked-output"),
                Stdio {
                    stdin: NeverRead,
                    stdout,
                    stderr: tokio::io::sink(),
                },
                CancellationToken::new(),
            )
            .await
        });
        use tokio::io::AsyncReadExt;
        receiver.read_u8().await.unwrap();
        server.close().await.unwrap();
        assert!(task.await.unwrap().is_err());
    })
    .await
    .unwrap();
}
