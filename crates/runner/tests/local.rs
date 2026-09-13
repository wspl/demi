use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use demi_command_service::{
    Client, Handler, InvocationContext, ServiceError,
    protocol::{Completion, Invocation, Record},
};
use demi_runner::commands::local::{self, Server};
use tokio_util::sync::CancellationToken;

struct Commands {
    cancelled: Arc<AtomicBool>,
}

impl Handler for Commands {
    fn operations(&self) -> Vec<String> {
        vec!["echo".into(), "wait".into()]
    }

    fn invoke(
        &self,
        mut context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let cancelled = self.cancelled.clone();
        Box::pin(async move {
            if context.request.operation == "wait" {
                context.output.stdout(Bytes::from_static(b"ready")).await?;
                context.cancellation.cancelled().await;
                cancelled.store(true, Ordering::SeqCst);
                return Err(ServiceError::Cancelled);
            }
            while let Some(bytes) = context.input.next().await? {
                context.output.stdout(bytes).await?;
            }
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }
}

fn invocation(operation: &str) -> Invocation {
    Invocation {
        operation: operation.into(),
        invocation_id: operation.into(),
        args: serde_json::json!({}),
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        env: BTreeMap::new(),
    }
}

#[tokio::test]
async fn private_endpoint_streams_binary_input_on_demand_and_joins_cancelled_calls() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let cancelled = Arc::new(AtomicBool::new(false));
        let server = Server::start(Arc::new(Commands {
            cancelled: cancelled.clone(),
        }))
        .await
        .unwrap();
        let endpoint = server.endpoint().to_owned();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&endpoint).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let socket = local::connect(&endpoint, &CancellationToken::new())
            .await
            .unwrap();
        let (client, connection) = Client::connect(socket).await.unwrap();
        let driver = tokio::spawn(connection);
        let (mut input, mut output) = client.invoke(&invocation("echo")).await.unwrap();
        assert_eq!(output.next().await.unwrap(), Some(Record::InputPull));
        input.write(Bytes::from_static(b"raw\0\xff")).await.unwrap();
        assert_eq!(
            output.next().await.unwrap(),
            Some(Record::Stdout(Bytes::from_static(b"raw\0\xff")))
        );
        assert_eq!(output.next().await.unwrap(), Some(Record::InputPull));
        input.end().unwrap();
        assert!(matches!(
            output.next().await.unwrap(),
            Some(Record::Completion(_))
        ));
        assert_eq!(output.next().await.unwrap(), None);
        drop(input);
        drop(output);
        let (input, mut output) = client.invoke(&invocation("wait")).await.unwrap();
        assert_eq!(
            output.next().await.unwrap(),
            Some(Record::Stdout(Bytes::from_static(b"ready")))
        );
        server.close().await.unwrap();
        assert!(cancelled.load(Ordering::SeqCst));
        #[cfg(unix)]
        assert!(!std::path::Path::new(&endpoint).exists());
        drop(input);
        drop(output);
        drop(client);
        // Server shutdown can surface a transport reset; the driver must still exit.
        let _transport_outcome = driver.await.unwrap();
    })
    .await
    .unwrap();
}
