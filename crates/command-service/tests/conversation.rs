use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use bytes::{BufMut, Bytes, BytesMut};
use demi_command_service::{
    Client, ConversationContext, Handler, InvocationContext, ServiceError,
    protocol::{Completion, ConversationRequest, Record},
    serve,
};
use tokio::sync::Notify;

struct Stateless;

impl Handler for Stateless {
    fn operations(&self) -> Vec<String> {
        vec!["noop".into()]
    }

    fn invoke(
        &self,
        _context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Box::pin(async {
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }
}

#[tokio::test]
async fn conversation_endpoint_has_no_grants_and_validates_release_identity() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (io, service) = tokio::io::duplex(4096);
        let server = tokio::spawn(serve(service, Arc::new(Stateless)));
        let (client, connection) = Client::connect(io).await.unwrap();
        let driver = tokio::spawn(connection);
        for (operation, conversation, body) in [
            ("status", None, b"{\"conversations\":[]}".as_slice()),
            ("release", Some("unknown"), b"{}".as_slice()),
            ("release", Some("unknown"), b"{}".as_slice()),
        ] {
            let (_input, mut output) = client
                .conversation(&ConversationRequest {
                    operation: operation.into(),
                    conversation: conversation.map(str::to_owned),
                })
                .await
                .unwrap();
            assert_eq!(
                output.next().await.unwrap(),
                Some(Record::Stdout(Bytes::copy_from_slice(body)))
            );
            assert_eq!(
                output.next().await.unwrap(),
                Some(Record::Completion(Completion {
                    exit_code: 0,
                    error: None
                }))
            );
            assert_eq!(output.next().await.unwrap(), None);
        }
        client.shutdown().await.unwrap();
        server.await.unwrap().unwrap();
        drop(client);
        driver.await.unwrap().unwrap();

        let (io, service) = tokio::io::duplex(4096);
        let server = tokio::spawn(serve(service, Arc::new(Stateless)));
        let (mut client, connection) = h2::client::handshake(io).await.unwrap();
        let driver = tokio::spawn(connection);
        for (path, body, status) in [
            ("/v1/resource", r#"{"operation":"acquire"}"#, 404),
            ("/v1/conversation", r#"{"operation":"release"}"#, 400),
            (
                "/v1/conversation",
                r#"{"operation":"acquire","conversation":"one"}"#,
                400,
            ),
            (
                "/v1/conversation",
                r#"{"operation":"release","conversation":""}"#,
                400,
            ),
            (
                "/v1/conversation",
                r#"{"operation":"status","resource":"one"}"#,
                400,
            ),
        ] {
            let (response, mut input) = client
                .send_request(
                    http::Request::post(format!("http://demi{path}"))
                        .body(())
                        .unwrap(),
                    false,
                )
                .unwrap();
            let mut bytes = BytesMut::new();
            bytes.put_u32(body.len() as u32);
            bytes.extend_from_slice(body.as_bytes());
            input.send_data(bytes.freeze(), true).unwrap();
            assert_eq!(response.await.unwrap().status(), status);
        }
        drop(client);
        driver.await.unwrap().unwrap();
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

struct Lifecycle {
    started: Arc<Notify>,
    cancelled: Arc<Notify>,
    closed: Arc<Notify>,
}

impl Handler for Lifecycle {
    fn operations(&self) -> Vec<String> {
        vec!["noop".into()]
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Stateless.invoke(context)
    }

    fn conversation(
        &self,
        context: ConversationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let started = self.started.clone();
        let cancelled = self.cancelled.clone();
        Box::pin(async move {
            started.notify_one();
            if context.request.conversation.as_deref() == Some("wait") {
                context.cancellation.cancelled().await;
                cancelled.notify_one();
                return Err(ServiceError::Cancelled);
            }
            Err(ServiceError::Handler("profile cleanup failed".into()))
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), ServiceError>> + Send>> {
        let closed = self.closed.clone();
        Box::pin(async move {
            closed.notify_one();
            Ok(())
        })
    }
}

#[tokio::test]
async fn conversation_cancellation_joins_hook_and_cleanup_failure_retires_service() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let started = Arc::new(Notify::new());
        let cancelled = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        let (io, service) = tokio::io::duplex(4096);
        let server = tokio::spawn(serve(service, Arc::new(Lifecycle { started: started.clone(), cancelled: cancelled.clone(), closed: closed.clone() })));
        let (client, connection) = Client::connect(io).await.unwrap();
        let driver = tokio::spawn(connection);
        let (mut input, output) = client.conversation(&ConversationRequest { operation: "release".into(), conversation: Some("wait".into()) }).await.unwrap();
        started.notified().await;
        input.cancel();
        cancelled.notified().await;
        drop(input);
        drop(output);
        assert_eq!(client.info().await.unwrap().operations, ["noop"]);
        let (_input, mut output) = client.conversation(&ConversationRequest { operation: "release".into(), conversation: Some("fail".into()) }).await.unwrap();
        // Cleanup failure closes the service; the final failure record may race
        // connection teardown, but it must never report successful completion.
        if let Ok(Some(Record::Completion(completion))) = output.next().await {
            assert_eq!(completion.exit_code, 1);
        }
        assert!(matches!(server.await.unwrap(), Err(ServiceError::ConversationCleanup(message)) if message.contains("profile cleanup failed")));
        closed.notified().await;
        drop(_input);
        drop(output);
        drop(client);
        // A fatal service exit may reset the parent connection.
        let _ = driver.await.unwrap();
    }).await.unwrap();
}
