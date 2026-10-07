//! A preview document's WebSockets over the stream (`preview.md` § The
//! forwarder and the relay).

use demi_command_package_browser_protocol::preview::{PreviewEngineMessage, PreviewRelayMessage};

use crate::support::{Frame, Relay, Site, client, engine, top};

#[tokio::test]
async fn a_socket_carries_messages_both_ways_and_closes_as_the_page_asks() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let page = top(&site.origin("www.site.test"));
    relay.send(&PreviewRelayMessage::SocketOpen {
        id: 7,
        environment: page,
        url: format!("ws://www.site.test:{}/socket", site.http),
        protocols: vec!["chat".into(), "other".into()],
        client: client(),
    });
    match relay.next().await {
        Frame::Control(PreviewEngineMessage::SocketOpened { id: 7, protocol, .. }) => assert_eq!(protocol, "chat"),
        other => panic!("unexpected {other:?}"),
    }
    relay.socket_message(7, false, "hello".as_bytes());
    relay.socket_message(7, true, &[0, 1, 2, 255]);
    for (binary, data) in [(false, &b"hello"[..]), (true, &[0, 1, 2, 255][..])] {
        match relay.next().await {
            Frame::Socket { id: 7, binary: echoed, data: echoed_data } => {
                assert_eq!((echoed, &echoed_data[..]), (binary, data));
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    relay.send(&PreviewRelayMessage::SocketClose {
        id: 7,
        code: 4000,
        reason: "done".into(),
    });
    match relay.next().await {
        Frame::Control(PreviewEngineMessage::SocketClose { id: 7, code, reason }) => {
            assert_eq!((code, reason.as_str()), (4000, "done"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn a_public_pages_socket_reaches_no_more_private_network() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    for host in ["127.0.0.1", "local.test"] {
        relay.send(&PreviewRelayMessage::SocketOpen {
            id: 1,
            environment: top(&site.origin("www.site.test")),
            url: format!("ws://{host}:{}/socket", site.http),
            protocols: Vec::new(),
            client: client(),
        });
        match relay.next().await {
            // A socket that cannot open reports an abnormal closure, as the
            // browser's does.
            Frame::Control(PreviewEngineMessage::SocketClose { id: 1, code, .. }) => assert_eq!(code, 1006, "{host}"),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(site.received().is_empty(), "{:?}", site.received());
}
