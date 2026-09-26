use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use bytes::Bytes;
use demi_command_service::protocol::{
    CommandCaller, CommandContext, CommandLocale, Completion, Invocation, Metadata, Record,
    RecordDecoder,
};
use demi_command_service::{Handler, InvocationContext, ServiceError, serve};
use http::Request;
use tokio::sync::Notify;

struct Fixture {
    cancelled: Arc<Notify>,
}

impl Handler for Fixture {
    type Metadata = Invocation;

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
        context: context(),
        json: None,
        edits: None,
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
        data.extend_from_slice(
            &demi_command_service::protocol::encode_input(Bytes::copy_from_slice(&binary)).unwrap(),
        );
        input.send_data(data.into(), true).unwrap();
        let records = response_records(echo.await.unwrap().into_body()).await;
        assert_eq!(
            records,
            vec![
                Record::InputPull,
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

#[tokio::test]
async fn sdk_client_keeps_other_calls_live_while_one_output_is_blocked() {
    use demi_command_service::Client;
    tokio::time::timeout(Duration::from_secs(5), async {
        let cancelled = Arc::new(Notify::new());
        let (client_io, server_io) = tokio::io::duplex(4096);
        let server = tokio::spawn(serve(
            server_io,
            Arc::new(Fixture {
                cancelled: cancelled.clone(),
            }),
        ));
        let (client, connection) = Client::connect(client_io).await.unwrap();
        let driver = tokio::spawn(connection);
        let request = |operation: &str| Invocation {
            context: context(),
            json: None,
            edits: None,
            operation: operation.into(),
            invocation_id: operation.into(),
            args: serde_json::json!({}),
            cwd: "/tmp".into(),
            env: BTreeMap::new(),
        };
        let (mut flood_input, mut flood_output) = client.invoke(&request("flood")).await.unwrap();
        flood_input.end().unwrap();
        assert!(matches!(
            flood_output.next().await.unwrap(),
            Some(Record::Stdout(_))
        ));
        // The producer has unlimited output. Leave the stream unread long enough
        // to exhaust its receive window before opening an independent call.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (mut input, mut output) = client.invoke(&request("echo")).await.unwrap();
        assert_eq!(output.next().await.unwrap(), Some(Record::InputPull));
        input
            .write(Bytes::from_static(b"independent"))
            .await
            .unwrap();
        assert_eq!(
            output.next().await.unwrap(),
            Some(Record::Stdout(Bytes::from_static(b"independent")))
        );
        input.end().unwrap();
        while output.next().await.unwrap().is_some() {}
        drop(input);
        drop(output);
        flood_input.cancel();
        cancelled.notified().await;
        drop(flood_input);
        drop(flood_output);
        client.shutdown().await.unwrap();
        drop(client);
        driver.await.unwrap().unwrap();
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

/// A service that answers each request and then resets it with NO_ERROR
/// while the caller is still sending it (RFC 9113 § 8.1): a conversation
/// request from its headers alone, an invocation after its metadata. Its
/// window of 16 bytes holds back the rest of what the caller sends.
async fn answer_early(io: tokio::io::DuplexStream) {
    let mut connection = h2::server::Builder::new()
        .initial_window_size(16)
        .handshake::<_, Bytes>(io)
        .await
        .unwrap();
    let mut answers = tokio::task::JoinSet::new();
    while let Some(accepted) = connection.accept().await {
        let (request, mut respond) = accepted.unwrap();
        answers.spawn(async move {
            let invocation = request.uri().path() == demi_command_service::protocol::INVOKE_PATH;
            let mut body = request.into_body();
            let mut metadata = Vec::new();
            // A four-byte length, then the metadata.
            while invocation
                && (metadata.len() < 4
                    || metadata.len() < 4 + u32::from_be_bytes(metadata[..4].try_into().unwrap()) as usize)
            {
                let chunk = body.data().await.unwrap().unwrap();
                body.flow_control().release_capacity(chunk.len()).unwrap();
                metadata.extend_from_slice(&chunk);
            }
            let mut stream = respond.send_response(http::Response::new(()), false).unwrap();
            let completion = Completion {
                exit_code: 0,
                error: None,
            };
            for record in [Record::Stdout(Bytes::from_static(b"{}")), Record::Completion(completion)] {
                stream.send_data(record.encode().unwrap(), false).unwrap();
            }
            stream.send_data(Bytes::new(), true).unwrap();
            // Dropping the request unread resets it with NO_ERROR once the
            // answer has left, as the service does.
        });
    }
}

/// A service may answer before it has the whole request and then reset it
/// with NO_ERROR (`native-runtime.md` § Request body and input demand): a
/// conversation release it answered succeeds, and an input chunk written
/// after its answer is dropped without an error.
#[tokio::test]
async fn what_a_caller_sends_after_an_early_answer_is_not_a_failure() {
    use demi_command_service::{Client, protocol::ConversationRequest};
    tokio::time::timeout(Duration::from_secs(5), async {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(answer_early(server_io));
        let (client, connection) = Client::connect(client_io).await.unwrap();
        let driver = tokio::spawn(connection);
        // The first answer brings the service's settings; the second request
        // leaves under its window.
        for conversation in ["first", "second"] {
            let (_input, mut output) = client
                .conversation(&ConversationRequest::Release {
                    conversation: conversation.into(),
                })
                .await
                .unwrap();
            assert_eq!(
                output.next().await.unwrap(),
                Some(Record::Stdout(Bytes::from_static(b"{}")))
            );
            assert!(matches!(output.next().await.unwrap(), Some(Record::Completion(_))));
            assert_eq!(output.next().await.unwrap(), None);
        }
        let (mut input, mut output) = client
            .invoke(&Invocation {
                context: context(),
                json: None,
                edits: None,
                operation: "echo".into(),
                invocation_id: "early".into(),
                args: serde_json::json!({}),
                cwd: "/tmp".into(),
                env: BTreeMap::new(),
            })
            .await
            .unwrap();
        while output.next().await.unwrap().is_some() {}
        input.write(Bytes::from(vec![0; 1024])).await.unwrap();
        input.end().unwrap();
        driver.abort();
        server.abort();
    })
    .await
    .unwrap();
}

fn context() -> CommandContext {
    CommandContext {
        conversation: "conversation".into(),
        caller: CommandCaller::agent("node"),
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
    }
}
