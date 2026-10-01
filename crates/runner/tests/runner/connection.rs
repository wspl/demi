//! The backend connection (`runner.md` § Connection and identity): the
//! runner's socket URL, its typed messages, a connection a malformed message
//! or the backend ends, and an inbound queue that holds the reading back
//! instead of closing the connection.

use std::{collections::BTreeMap, time::Duration};

use demi_backend_remote_host::testing::{RunnerProcess, RunnerProcessOptions};
use demi_runner_protocol::wire::{self, Inbound, Outbound};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{Request, Response},
};

use crate::Host;

/// The runner connects to `/api/runner` of a backend URL that names no path,
/// and to the path and query of one that names them.
#[tokio::test]
async fn the_socket_is_the_backend_urls_path_or_the_runner_route() {
    tokio::time::timeout(Duration::from_secs(60), async {
        for (path, requested) in [("", "/api/runner"), ("/custom?x=1", "/custom?x=1")] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let backend = format!("http://{}{path}", listener.local_addr().unwrap());
            let mut process = RunnerProcess::start(
                &backend,
                RunnerProcessOptions {
                    token: Some("test-token".into()),
                    ..RunnerProcessOptions::default()
                },
            );
            let (stream, _) = listener.accept().await.unwrap();
            let mut uri = None;
            let socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &Request, response: Response| {
                    uri = Some(request.uri().to_string());
                    Ok(response)
                },
            )
            .await
            .unwrap();
            assert_eq!(uri.as_deref(), Some(requested), "{backend}");
            drop(socket);
            process.stop().await;
        }
    })
    .await
    .unwrap();
}

/// A ping is answered with a pong that counts the jobs; a connection the
/// backend closes is opened again.
#[tokio::test]
async fn typed_exchange_and_remote_close() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        host.send(Inbound::Ping {}).await;
        assert!(matches!(host.frame().await, Outbound::Pong { jobs: 0 }));
        host.close_connection().await;
        host.reconnected().await;
        host.close().await;
    })
    .await
    .unwrap();
}

/// A message that is not the protocol's ends the connection; the runner
/// connects again.
#[tokio::test]
async fn malformed_input_fails_the_connection() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        host.send_raw(Message::Text("{}".into())).await;
        host.ended().await;
        host.reconnected().await;
        host.close().await;
    })
    .await
    .unwrap();
}

/// Far more messages than the runner's inbound queue holds, sent while the
/// backend reads nothing, are each answered: a full queue holds the reading
/// back instead of closing the connection.
#[tokio::test]
async fn a_full_inbound_queue_waits_instead_of_closing() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        let ping = wire::encode(&Inbound::Ping {}).unwrap().into_bytes();
        for _ in 0..256 {
            host.send_raw(Message::Binary(ping.clone().into())).await;
        }
        for _ in 0..256 {
            assert!(matches!(host.frame().await, Outbound::Pong { .. }));
        }
        host.close().await;
    })
    .await
    .unwrap();
}

impl Host {
    async fn send_raw(&mut self, message: Message) {
        self.socket.send(message).await.unwrap();
    }

    /// Closes the connection from the backend's end.
    async fn close_connection(&mut self) {
        self.socket.close(None).await.unwrap();
        self.ended().await;
    }

    /// Waits until the runner's end of the connection is gone.
    async fn ended(&mut self) {
        while let Some(Ok(_)) = self.socket.next().await {}
    }
}
