use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use bytes::Bytes;
use demi_command_service::protocol::{Completion, Invocation, Record, RecordDecoder};
use demi_command_service::{Handler, InvocationContext, ServiceError, serve};
use http::Request;
use tokio::sync::Notify;

struct Fixture {
    cancelled: Arc<Notify>,
}

impl Handler for Fixture {
    fn operations(&self) -> Vec<String> {
        vec!["echo".into(), "wait".into(), "flood".into()]
    }

    fn invoke(
        &self,
        mut context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let cancelled = self.cancelled.clone();
        Box::pin(async move {
            match context.request.operation.as_str() {
                "wait" => {
                    context.cancellation.cancelled().await;
                    cancelled.notify_one();
                    return Err(ServiceError::Cancelled);
                }
                "flood" => loop {
                    if context
                        .output
                        .stdout(Bytes::from(vec![42; 64 * 1024]))
                        .await
                        .is_err()
                    {
                        cancelled.notify_one();
                        return Err(ServiceError::Cancelled);
                    }
                },
                _ => {
                    while let Some(bytes) = context.input.next().await? {
                        context.output.stdout(bytes).await?;
                    }
                    context.output.stderr(Bytes::from_static(b"done")).await?;
                }
            }
            Ok(Completion {
                exit_code: 7,
                error: None,
            })
        })
    }
}

fn invocation(operation: &str) -> Bytes {
    Invocation {
        operation: operation.into(),
        invocation_id: operation.into(),
        args: serde_json::json!({}),
        cwd: "/tmp".into(),
        env: BTreeMap::new(),
    }
    .encode()
    .unwrap()
}

async fn response_records(mut body: h2::RecvStream) -> Vec<Record> {
    let mut decoder = RecordDecoder::default();
    let mut records = Vec::new();
    while let Some(bytes) = body.data().await {
        let mut bytes = bytes.unwrap();
        let length = bytes.len();
        while !bytes.is_empty() {
            if let Some(record) = decoder.decode(&mut bytes).unwrap() {
                records.push(record);
            }
        }
        body.flow_control().release_capacity(length).unwrap();
    }
    decoder.finish().unwrap();
    records
}

#[tokio::test]
async fn concurrent_binary_echo_and_cancel_preserve_connection() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let cancelled = Arc::new(Notify::new());
        let (client_io, server_io) = tokio::io::duplex(4096);
        let server = tokio::spawn(serve(
            server_io,
            Arc::new(Fixture {
                cancelled: cancelled.clone(),
            }),
        ));
        let (mut client, connection) = h2::client::handshake(client_io).await.unwrap();
        let driver = tokio::spawn(connection);
        let request = || Request::post("https://demi/v1/invoke").body(()).unwrap();
        let (waiting_response, mut waiting_input) = client.send_request(request(), false).unwrap();
        waiting_input.send_data(invocation("wait"), true).unwrap();
        let waiting_response = waiting_response.await.unwrap();
        assert_eq!(waiting_response.status(), 200);

        let (echo, mut input) = client.send_request(request(), false).unwrap();
        // Metadata and stdin share a DATA frame; input must retain the suffix.
        let binary = [0, 255, 13, 10, 128];
        let mut data = invocation("echo").to_vec();
        data.extend_from_slice(&binary);
        input.send_data(data.into(), true).unwrap();
        let records = response_records(echo.await.unwrap().into_body()).await;
        assert_eq!(
            records,
            vec![
                Record::Stdout(Bytes::copy_from_slice(&binary)),
                Record::Stderr(Bytes::from_static(b"done")),
                Record::Completion(Completion {
                    exit_code: 7,
                    error: None
                }),
            ]
        );
        waiting_input.send_reset(h2::Reason::CANCEL);
        cancelled.notified().await;
        drop(waiting_response);
        drop(waiting_input);
        drop(input);
        drop(client);
        driver.await.unwrap().unwrap();
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reset_interrupts_flow_control_blocked_output() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let cancelled = Arc::new(Notify::new());
        let (client_io, server_io) = tokio::io::duplex(4096);
        let server = tokio::spawn(serve(
            server_io,
            Arc::new(Fixture {
                cancelled: cancelled.clone(),
            }),
        ));
        let (mut client, connection) = h2::client::handshake(client_io).await.unwrap();
        let driver = tokio::spawn(connection);
        let request = Request::post("https://demi/v1/invoke").body(()).unwrap();
        let (response, mut input) = client.send_request(request, false).unwrap();
        input.send_data(invocation("flood"), true).unwrap();
        let response = response.await.unwrap();
        let mut body = response.into_body();
        // Exhaust the default stream window without returning any capacity.
        let mut received = 0;
        while received < 65_535 {
            received += body.data().await.unwrap().unwrap().len();
        }
        input.send_reset(h2::Reason::CANCEL);
        cancelled.notified().await;
        drop(body);
        drop(input);
        drop(client);
        driver.await.unwrap().unwrap();
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}
