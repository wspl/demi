#![cfg(unix)]

use std::{
    collections::BTreeMap, future::Future, os::fd::OwnedFd, pin::Pin, sync::Arc, time::Duration,
};

use bytes::Bytes;
use demi_command_service::protocol::{Completion, Invocation, Record, RecordDecoder};
use demi_command_service::{Handler, InvocationContext, ServiceError, serve};
use http::Request;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{UnixStream, unix::pipe},
};

struct Echo;

impl Handler for Echo {
    fn operations(&self) -> Vec<String> {
        vec!["echo".into()]
    }

    fn invoke(
        &self,
        mut context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Box::pin(async move {
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

async fn exchange<C, S>(client_io: C, server_io: S)
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::time::timeout(Duration::from_secs(5), async move {
        let server = tokio::spawn(serve(server_io, Arc::new(Echo)));
        let (mut client, connection) = h2::client::handshake(client_io).await.unwrap();
        let driver = tokio::spawn(connection);
        let request = Request::post("https://demi/v1/invoke").body(()).unwrap();
        let (response, mut input) = client.send_request(request, false).unwrap();
        input
            .send_data(
                Invocation {
                    operation: "echo".into(),
                    invocation_id: "transport".into(),
                    args: serde_json::json!({}),
                    cwd: "/tmp".into(),
                    env: BTreeMap::new(),
                }
                .encode()
                .unwrap(),
                false,
            )
            .unwrap();
        let mut body = response.await.unwrap().into_body();
        input
            .send_data(Bytes::from_static(b"live\0\xff"), false)
            .unwrap();
        let mut decoder = RecordDecoder::default();
        let mut records = Vec::new();
        while records.is_empty() {
            let mut bytes = body.data().await.unwrap().unwrap();
            let length = bytes.len();
            while !bytes.is_empty() {
                if let Some(record) = decoder.decode(&mut bytes).unwrap() {
                    records.push(record);
                }
            }
            body.flow_control().release_capacity(length).unwrap();
        }
        // The response arrives before stdin EOF, proving full duplex live input.
        assert_eq!(records, [Record::Stdout(Bytes::from_static(b"live\0\xff"))]);
        input.send_data(Bytes::new(), true).unwrap();
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
        assert_eq!(
            records.last(),
            Some(&Record::Completion(Completion {
                exit_code: 0,
                error: None
            }))
        );
        drop(body);
        drop(input);
        let shutdown = Request::post("https://demi/v1/shutdown").body(()).unwrap();
        let (response, shutdown_input) = client.send_request(shutdown, true).unwrap();
        assert_eq!(response.await.unwrap().status(), 200);
        drop(shutdown_input);
        server.await.unwrap().unwrap();
        drop(client);
        driver.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn http2_over_unix_socket_streams_before_eof_and_shuts_down() {
    let (client, server) = UnixStream::pair().unwrap();
    exchange(client, server).await;
}

#[tokio::test]
async fn http2_over_two_stdio_pipes_streams_before_eof_and_shuts_down() {
    let (request_read, request_write) = std::io::pipe().unwrap();
    let (response_read, response_write) = std::io::pipe().unwrap();
    let client = tokio::io::join(
        pipe::Receiver::from_owned_fd(OwnedFd::from(response_read)).unwrap(),
        pipe::Sender::from_owned_fd(OwnedFd::from(request_write)).unwrap(),
    );
    let server = tokio::io::join(
        pipe::Receiver::from_owned_fd(OwnedFd::from(request_read)).unwrap(),
        pipe::Sender::from_owned_fd(OwnedFd::from(response_write)).unwrap(),
    );
    exchange(client, server).await;
}
