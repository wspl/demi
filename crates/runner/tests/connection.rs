use std::time::Duration;

use demi_runner::connection::wire;
use demi_runner::connection::{Connection, socket_url};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, protocol::Role},
};
use tokio_util::sync::CancellationToken;

async fn pair() -> (Connection, WebSocketStream<tokio::io::DuplexStream>) {
    let (client, server) = tokio::io::duplex(1024);
    let client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    (
        Connection::from_socket(client, CancellationToken::new()),
        server,
    )
}

#[test]
fn backend_url_preserves_explicit_path_and_query() {
    assert_eq!(
        socket_url("https://example.test").unwrap().as_str(),
        "wss://example.test/api/runner"
    );
    assert_eq!(
        socket_url("http://example.test/custom?x=1")
            .unwrap()
            .as_str(),
        "ws://example.test/custom?x=1"
    );
    assert!(socket_url("https://user:pass@example.test").is_err());
}

#[tokio::test]
async fn typed_exchange_and_remote_close() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (mut client, mut server) = pair().await;
        let ping = rmp_serde::to_vec_named(&serde_json::json!({"type":"ping"})).unwrap();
        server.send(Message::Binary(ping.into())).await.unwrap();
        assert!(matches!(
            client.input.recv().await.unwrap(),
            wire::Inbound::Ping {}
        ));
        client.output.send(wire::pong(2).unwrap()).await.unwrap();
        let response = server.next().await.unwrap().unwrap().into_data();
        let value: serde_json::Value = rmp_serde::from_slice(&response).unwrap();
        assert_eq!(value, serde_json::json!({"type":"pong", "jobs":2}));
        server.close(None).await.unwrap();
        assert!(client.input.recv().await.is_none());
        client.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn close_interrupts_stalled_output() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (client, _server) = pair().await;
        let output = wire::spawn_output(
            "stalled".into(),
            "stdout".into(),
            wire::WireBytes(vec![0; 65536]),
        )
        .unwrap();
        client.output.send(output).await.unwrap();
        tokio::task::yield_now().await;
        client.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn malformed_input_fails_connection() {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (mut client, mut server) = pair().await;
        server.send(Message::Text("{}".into())).await.unwrap();
        assert!(client.input.recv().await.is_none());
        assert!(client.close().await.is_err());
    })
    .await
    .unwrap();
}
