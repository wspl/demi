#![cfg(unix)]

use std::{
    collections::BTreeMap, future::Future, os::fd::OwnedFd, pin::Pin, sync::Arc, time::Duration,
};

use bytes::Bytes;
use demi_command_service::protocol::{Completion, Invocation, Record};
use demi_command_service::{Client, Handler, InvocationContext, ServiceError, serve};
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
        let (client, connection) = Client::connect(client_io).await.unwrap();
        let driver = tokio::spawn(connection);
        let (mut input, mut output) = client
            .invoke(&Invocation {
                edits: None,
                operation: "echo".into(),
                invocation_id: "transport".into(),
                args: serde_json::json!({}),
                cwd: "/tmp".into(),
                env: BTreeMap::new(),
            })
            .await
            .unwrap();
        assert_eq!(output.next().await.unwrap(), Some(Record::InputPull));
        input
            .write(Bytes::from_static(b"live\0\xff"))
            .await
            .unwrap();
        // The response arrives before stdin EOF, proving full duplex live input.
        assert_eq!(
            output.next().await.unwrap(),
            Some(Record::Stdout(Bytes::from_static(b"live\0\xff")))
        );
        assert_eq!(output.next().await.unwrap(), Some(Record::InputPull));
        input.end().unwrap();
        assert_eq!(
            output.next().await.unwrap(),
            Some(Record::Completion(Completion {
                exit_code: 0,
                error: None
            }))
        );
        assert_eq!(output.next().await.unwrap(), None);
        drop(input);
        drop(output);
        client.shutdown().await.unwrap();
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
