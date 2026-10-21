//! The backend connection (`crates/runner/src/connection/mod.rs`): its typed
//! messages, a connection a malformed message or the backend ends, and an
//! inbound queue that holds the reading back instead of closing the
//! connection.

use std::{collections::BTreeMap, time::Duration};

use demi_runner_protocol::wire::{Inbound, Outbound};
use tokio_tungstenite::tungstenite::{
    Message,
    protocol::{CloseFrame, frame::coding::CloseCode},
};

use crate::Host;

/// A request the runner answers at once, under `id`.
fn exists(id: &str) -> Inbound {
    Inbound::FsExists {
        id: id.into(),
        path: ".".into(),
        cwd: None,
    }
}

/// The runner reports its job count when it is online; a connection the
/// backend closes is opened again.
#[tokio::test]
async fn typed_exchange_and_remote_close() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        // The runner reports its job count once it is online.
        assert!(matches!(
            host.frame_with_counts().await,
            Outbound::JobCount { jobs: 0 }
        ));
        host.send(exists("a")).await;
        assert!(matches!(host.frame().await, Outbound::FsOk(_)));
        host.close_connection().await;
        host.reconnected().await;
        host.close().await;
    })
    .await
    .unwrap();
}

/// Ending the last tree subscription must not stop the runner's control
/// loop. A new subscription, a ping and a reconnect still work, and closing
/// with a live watch releases its threads too. About 1 s; no model calls.
#[tokio::test]
async fn ending_a_tree_watch_keeps_the_runner_responsive() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        let path = host.home().to_string_lossy().into_owned();
        for id in ["first", "second"] {
            host.send(Inbound::FsSubscribe {
                id: id.into(),
                kind: demi_runner_protocol::files::SubscriptionKind::Worktree,
                path: path.clone(),
                suffix: None,
            })
            .await;
            assert!(matches!(host.frame().await, Outbound::FsLive { id: live, .. } if live == id));
            host.send(Inbound::FsUnsubscribe { id: id.into() }).await;
        }
        // A new subscription is handled by the same owner after both ends,
        // unlike a ping that could be answered before it processes them.
        host.send(Inbound::FsSubscribe {
            id: "last".into(),
            kind: demi_runner_protocol::files::SubscriptionKind::Worktree,
            path,
            suffix: None,
        })
        .await;
        assert!(matches!(host.frame().await, Outbound::FsLive { id, .. } if id == "last"));
        host.send(exists("b")).await;
        assert!(matches!(host.frame().await, Outbound::FsOk(_)));
        host.close_connection().await;
        host.reconnected().await;
        host.welcome(&[crate::CONVERSATION]).await;
        host.send(exists("c")).await;
        assert!(matches!(host.frame().await, Outbound::FsOk(_)));
        host.close().await;
    })
    .await
    .expect("ending a tree watch must not hang the runner");
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

/// A connection the backend closes over a message of the runner's that it
/// cannot decode ends with the backend's reason, which the runner's log
/// reports; the runner connects again (`crates/runner/src/connection/mod.rs`). About 0.5 s: a runner's start and a reconnect.
#[tokio::test]
async fn a_backend_that_cannot_decode_a_message_ends_the_connection_with_its_reason() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        host.send_raw(Message::Close(Some(CloseFrame {
            code: CloseCode::Invalid,
            reason: "cannot decode job_count: unknown field `load`".into(),
        })))
        .await;
        host.ended().await;
        host.reconnected().await;
        let log = host.process.output();
        assert!(
            log.contains("connection ended: the backend cannot decode job_count: unknown field `load`"),
            "{log}"
        );
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
        for index in 0..256 {
            host.send(exists(&format!("q{index}"))).await;
        }
        for _ in 0..256 {
            assert!(matches!(host.frame().await, Outbound::FsOk(_)));
        }
        host.close().await;
    })
    .await
    .unwrap();
}

impl Host {
    async fn send_raw(&mut self, message: Message) {
        self.socket.send_raw(message).await;
    }

    /// Closes the connection from the backend's end.
    async fn close_connection(&mut self) {
        self.socket.close().await;
        self.ended().await;
    }

    /// Waits until the runner's end of the connection is gone.
    pub(crate) async fn ended(&mut self) {
        while let Some(Ok(_)) = self.socket.next().await {}
    }
}
