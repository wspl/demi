//! The backend's client of the machine manager's socket (`managed-hosts.md`
//! § Control and ownership), against a peer the tests script.

use std::path::PathBuf;
use std::time::Duration;

use demi_backend_cloud::client::{MachinesClient, MachinesError};
use demi_machine_manager_protocol::{
    BaseVersion, CurrentBaseVersionParams, HibernateParams, ImageStateParams, MachineCall,
    MachineImageState, MachineRequest, MachineResponse, decode_request, encode_line,
};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixListener;
use tokio::net::unix::OwnedWriteHalf;

/// One connection of a peer the test scripts: the requests it read, and
/// the lines it writes back.
struct Peer {
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    writer: OwnedWriteHalf,
}

impl Peer {
    async fn accept(listener: &UnixListener) -> Self {
        let (stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("the client connects")
            .unwrap();
        let (read, writer) = stream.into_split();
        Self {
            lines: BufReader::new(read).lines(),
            writer,
        }
    }

    async fn request(&mut self) -> MachineRequest {
        let line = self.lines.next_line().await.unwrap().expect("a request");
        decode_request(&line).unwrap()
    }

    async fn write(&mut self, response: &MachineResponse) {
        self.writer.write_all(&encode_line(response)).await.unwrap();
    }

    async fn write_raw(&mut self, line: &str) {
        self.writer.write_all(line.as_bytes()).await.unwrap();
    }

    async fn ok(&mut self, id: &str, result: serde_json::Value) {
        let response = MachineResponse::Ok {
            id: id.into(),
            result,
        };
        self.write(&response).await;
    }

    /// Whether the client closed the connection.
    async fn closed(&mut self) -> bool {
        tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
            .await
            .is_ok_and(|line| matches!(line, Ok(None)))
    }
}

fn socket() -> (tempfile::TempDir, PathBuf, UnixListener) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("machines.sock");
    let listener = UnixListener::bind(&path).unwrap();
    (directory, path, listener)
}

fn device(name: &str) -> String {
    name.to_owned()
}

#[tokio::test]
async fn calls_cross_the_socket_and_their_replies_are_matched_by_id_whatever_their_order() {
    let (_directory, path, listener) = socket();
    let (client, _deaths) = MachinesClient::new(path);
    let client = std::sync::Arc::new(client);
    // Nothing connects before the first call.
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    let calls = {
        let client = client.clone();
        tokio::spawn(async move {
            tokio::join!(
                client.call(CurrentBaseVersionParams {}),
                client.call(ImageStateParams {
                    device_id: device("dev-1")
                }),
            )
        })
    };
    let mut peer = Peer::accept(&listener).await;
    let first = peer.request().await;
    let second = peer.request().await;
    assert_eq!((first.id.as_str(), second.id.as_str()), ("1", "2"));
    let state = MachineImageState {
        generation: demi_machine_manager_protocol::GenerationId::parse("gen-1").unwrap(),
        base_version: BaseVersion::parse("base-1").unwrap(),
        reset_id: None,
        system_bytes: 1024.try_into().unwrap(),
        home_bytes: 2048.try_into().unwrap(),
    };
    // The later request is answered first.
    let (base, image) = match (&first.call, &second.call) {
        (MachineCall::CurrentBaseVersion(_), MachineCall::ImageState(_)) => (&first.id, &second.id),
        (MachineCall::ImageState(_), MachineCall::CurrentBaseVersion(_)) => (&second.id, &first.id),
        other => panic!("{other:?}"),
    };
    peer.ok(image, serde_json::to_value(&state).unwrap()).await;
    peer.ok(base, serde_json::json!("base-1")).await;
    let (base, image) = calls.await.unwrap();
    assert_eq!(base.unwrap().as_str(), "base-1");
    assert_eq!(image.unwrap(), Some(state));
}

#[tokio::test]
async fn a_failure_reply_fails_its_call_and_the_connection_stays_usable() {
    let (_directory, path, listener) = socket();
    let (client, _deaths) = MachinesClient::new(path);
    let client = std::sync::Arc::new(client);
    let failing = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .call(HibernateParams {
                    device_id: device("dev-9"),
                })
                .await
        })
    };
    let mut peer = Peer::accept(&listener).await;
    let request = peer.request().await;
    let refusal = MachineResponse::Error {
        id: request.id,
        message: "no such machine".into(),
    };
    peer.write(&refusal).await;
    assert_eq!(
        failing.await.unwrap(),
        Err(MachinesError::Failed("no such machine".into()))
    );
    // Lines that are not replies, or answer nothing asked, are dropped.
    peer.write_raw("not json\n{\"type\":\"ok\",\"id\":\"99\",\"result\":null}\n\n")
        .await;
    let answered = {
        let client = client.clone();
        tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
    };
    let request = peer.request().await;
    peer.ok(&request.id, serde_json::json!("base-1")).await;
    assert_eq!(answered.await.unwrap().unwrap().as_str(), "base-1");
    // A result the operation does not have fails its call.
    let odd = {
        let client = client.clone();
        tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
    };
    let request = peer.request().await;
    peer.ok(&request.id, serde_json::json!(7)).await;
    assert!(matches!(
        odd.await.unwrap(),
        Err(MachinesError::Result { .. })
    ));
}

#[tokio::test]
async fn a_death_reaches_the_router_and_close_reconciles_then_disconnects_until_the_next_call() {
    let (_directory, path, listener) = socket();
    let (client, mut deaths) = MachinesClient::new(path);
    let client = std::sync::Arc::new(client);
    // A client that never connected closes without a word.
    client.close().await.unwrap();
    let first = {
        let client = client.clone();
        tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
    };
    let mut peer = Peer::accept(&listener).await;
    let request = peer.request().await;
    peer.write(&MachineResponse::Death {
        device_id: "dev-1".into(),
    })
    .await;
    peer.ok(&request.id, serde_json::json!("base-1")).await;
    first.await.unwrap().unwrap();
    assert_eq!(deaths.recv().await.unwrap().as_str(), "dev-1");

    let closing = {
        let client = client.clone();
        tokio::spawn(async move { client.close().await })
    };
    let reconcile = peer.request().await;
    assert!(
        matches!(reconcile.call, MachineCall::Reconcile(_)),
        "{reconcile:?}"
    );
    peer.ok(&reconcile.id, serde_json::Value::Null).await;
    closing.await.unwrap().unwrap();
    assert!(peer.closed().await);

    let again = {
        let client = client.clone();
        tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
    };
    let mut peer = Peer::accept(&listener).await;
    let request = peer.request().await;
    peer.ok(&request.id, serde_json::json!("base-2")).await;
    assert_eq!(again.await.unwrap().unwrap().as_str(), "base-2");
}

#[tokio::test]
async fn a_manager_that_goes_away_fails_the_calls_in_flight_and_is_dialed_again() {
    let (directory, path, listener) = socket();
    let (client, _deaths) = MachinesClient::new(path.clone());
    let client = std::sync::Arc::new(client);
    let in_flight = {
        let client = client.clone();
        tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
    };
    let mut peer = Peer::accept(&listener).await;
    peer.request().await;
    drop(peer);
    drop(listener);
    let failed = in_flight.await.unwrap().unwrap_err();
    assert!(
        failed
            .to_string()
            .starts_with("Machine manager unavailable during current_base_version"),
        "{failed}"
    );

    // With no manager listening, a call fails at once.
    std::fs::remove_file(&path).unwrap();
    let refused = client.call(CurrentBaseVersionParams {}).await.unwrap_err();
    assert!(
        matches!(refused, MachinesError::Unavailable { .. }),
        "{refused}"
    );

    let listener = UnixListener::bind(directory.path().join("machines.sock")).unwrap();
    let later = {
        let client = client.clone();
        tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
    };
    let mut peer = Peer::accept(&listener).await;
    let request = peer.request().await;
    peer.ok(&request.id, serde_json::json!("base-1")).await;
    assert_eq!(later.await.unwrap().unwrap().as_str(), "base-1");
}
