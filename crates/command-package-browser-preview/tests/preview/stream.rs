//! The stream itself (`preview.md` § The stream): bodies move one chunk per
//! pull both ways, the browser's cancellation reaches upstream, the engine
//! names the labels a runtime registers, page states move, and a frame the
//! protocol refuses ends the stream.

use std::sync::Arc;
use std::time::Duration;

use bytes::{BufMut, BytesMut};
use demi_command_package_browser_preview::{PageStates, StreamError, TakenState};
use demi_command_package_browser_protocol::preview::{
    BODY_CHUNK_BYTES, CHUNK_FRAME, CONTROL_FRAME, MAX_STORAGE_BYTES, PageStorage, PreviewEngineMessage, StorageItem, PreviewMode,
    PreviewOpenInput, PreviewRelayMessage, PreviewRequest, PreviewScheme, SOCKET_MESSAGE_FRAME,
};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

use crate::support::{DOMAIN, Frame, HOST, NAMESPACE, Relay, Site, client, echoed_header, engine, opening, request, top};

#[tokio::test]
async fn bodies_move_one_chunk_per_pull() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let page = top(&site.origin("www.site.test"));
    let body = vec![b'x'; 3 * BODY_CHUNK_BYTES + 10];
    let id = relay.id();
    relay.send(&PreviewRelayMessage::Request {
        id,
        environment: page.clone(),
        request: PreviewRequest {
            method: "POST".into(),
            body: true,
            ..request(&site.https("www.site.test", "/echo"), PreviewMode::Cors, "", Some(page.clone()))
        },
        client: client(),
    });
    // The engine asks for the request's body chunk by chunk; meanwhile
    // another request of the stream is answered whole, and nothing more of
    // the first is asked for or sent.
    let mut chunks = body.chunks(BODY_CHUNK_BYTES).chain([&[][..]]);
    let mut pulled = 0;
    let (head, answered) = loop {
        match relay.next().await {
            Frame::Control(PreviewEngineMessage::Pull { id: pulled_id }) if pulled_id == id => {
                pulled += 1;
                if pulled == 1 {
                    let other = relay.fetch(page.clone(), opening(&site.https("www.site.test", "/echo"))).await.unwrap();
                    assert_eq!(echoed_header(&other.echoed(), "sec-fetch-mode"), Some("navigate"));
                }
                relay.request_body(id, chunks.next().unwrap());
            }
            Frame::Control(PreviewEngineMessage::Response { id: answered, headers, .. }) if answered == id => {
                break (headers, pulled);
            }
            other => panic!("unexpected {other:?}"),
        }
    };
    assert_eq!(answered, 5, "{head:?}");
    // The answer, as long as the body echoed, comes one chunk per pull: a
    // chunk the relay did not ask for would arrive before the other
    // request's answer.
    let mut received = Vec::new();
    loop {
        relay.send(&PreviewRelayMessage::Pull { id });
        let data = match relay.next().await {
            Frame::Chunk { id: chunked, data } if chunked == id => data,
            other => panic!("unexpected {other:?}"),
        };
        assert!(data.len() <= BODY_CHUNK_BYTES);
        if data.is_empty() {
            break;
        }
        if received.is_empty() {
            relay.fetch(page.clone(), opening(&site.https("www.site.test", "/echo"))).await.unwrap();
        }
        received.extend_from_slice(&data);
    }
    let echoed: serde_json::Value = serde_json::from_slice(&received).unwrap();
    assert_eq!(echoed["body"].as_str().unwrap().len(), body.len());
}

#[tokio::test]
async fn the_browsers_cancellation_ends_the_request_upstream() {
    let directory = tempfile::tempdir().unwrap();
    // A server that never answers.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let relay = Relay::open(engine(&directory));
    relay.send(&PreviewRelayMessage::Request {
        id: 3,
        environment: top(&origin),
        request: opening(&format!("{origin}/slow")),
        client: client(),
    });
    let (mut connection, _) = listener.accept().await.unwrap();
    let mut head = [0; 64];
    assert!(connection.read(&mut head).await.unwrap() > 0);
    relay.send(&PreviewRelayMessage::Cancel { id: 3 });
    // The engine closes its connection: the server reads its end.
    let mut rest = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), connection.read_to_end(&mut rest))
        .await
        .expect("the engine closes the connection")
        .unwrap();
    // And the stream goes on.
    relay.end().await.unwrap();
}

/// The relay registers only the labels the engine computes for the
/// environments a runtime names: each is the label the engine opens that
/// environment's address under, so a registered label leads to the page it
/// stands for.
#[tokio::test]
async fn the_engine_names_the_labels_of_the_environments_a_runtime_registers() {
    let directory = tempfile::tempdir().unwrap();
    let mut relay = Relay::open(engine(&directory));
    let opened = |url: &str| {
        demi_command_package_browser_preview::opening(&PreviewOpenInput {
            url: url.into(),
            scheme: PreviewScheme::Https,
            domain: DOMAIN.into(),
            namespace: NAMESPACE.into(),
            host: HOST.into(),
        })
        .unwrap()
    };
    let app = opened("http://localhost:5173/");
    let docs = opened("https://docs.site.test/guide");
    let id = relay.id();
    relay.send(&PreviewRelayMessage::Labels {
        id,
        environments: vec![app.environment.clone(), docs.environment.clone()],
    });
    let Frame::Control(PreviewEngineMessage::Labels { id: answered, labels }) = relay.next().await else {
        panic!("the labels' answer");
    };
    assert_eq!(answered, id);
    assert_eq!(labels, [(app.label, app.environment), (docs.label, docs.environment)].into_iter().collect());
}

#[tokio::test]
async fn a_frame_the_protocol_refuses_ends_the_stream() {
    let directory = tempfile::tempdir().unwrap();
    let engine = engine(&directory);
    let framed = |kind: u8, payload: &[u8]| {
        let mut bytes = BytesMut::new();
        bytes.put_u32(1 + payload.len() as u32);
        bytes.put_u8(kind);
        bytes.put_slice(payload);
        bytes.freeze()
    };
    let hello = serde_json::to_vec(&PreviewRelayMessage::Hello {
        scheme: PreviewScheme::Https,
        domain: "preview.test".into(),
        namespace: "k3f9a2ab".into(),
        host: "host-1".into(),
    })
    .unwrap();
    let pull = serde_json::to_vec(&PreviewRelayMessage::Pull { id: 1 }).unwrap();
    let cases: Vec<(&str, Vec<bytes::Bytes>)> = vec![
        ("a request before hello", vec![framed(CONTROL_FRAME, &pull)]),
        ("a second hello", vec![framed(CONTROL_FRAME, &hello), framed(CONTROL_FRAME, &hello)]),
        ("an empty frame", vec![bytes::Bytes::from_static(&[0, 0, 0, 0])]),
        ("a frame too large", vec![bytes::Bytes::from_static(&[2, 0, 0, 0])]),
        ("not JSON", vec![framed(CONTROL_FRAME, b"{")]),
        ("an engine's message", vec![framed(CONTROL_FRAME, br#"{"type":"failed","id":1,"reason":""}"#)]),
        ("an engine's frame kind", vec![framed(CONTROL_FRAME, &hello), framed(CHUNK_FRAME, &[0, 0, 0, 1])]),
        (
            "a text message that is not UTF-8",
            vec![framed(CONTROL_FRAME, &hello), framed(SOCKET_MESSAGE_FRAME, &[0, 0, 0, 1, 0, 0xff])],
        ),
        ("a frame cut short", vec![framed(CONTROL_FRAME, &hello), bytes::Bytes::from_static(&[0, 0, 0, 9, 1])]),
    ];
    for (case, frames) in cases {
        let relay = Relay::unopened(engine.clone());
        for frame in frames {
            relay.raw(frame);
        }
        let ended = relay.end().await;
        assert!(matches!(ended, Err(StreamError::Protocol(_))), "{case}: {ended:?}");
    }
}

#[tokio::test]
async fn a_body_for_a_request_without_one_ends_the_stream() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let page = top(&site.origin("www.site.test"));
    relay.send(&PreviewRelayMessage::Request {
        id: 1,
        environment: page.clone(),
        request: request(&site.https("www.site.test", "/echo"), PreviewMode::Cors, "", Some(page)),
        client: client(),
    });
    relay.request_body(1, b"unasked");
    assert!(matches!(relay.ended().await, Err(StreamError::Protocol(_))));
}

/// A conversation's browser whose tab `t1` shows a signed-in page, and which
/// keeps what the user's browser hands it.
struct SignedIn {
    storage: PageStorage,
    kept: std::sync::Mutex<Vec<(String, Vec<String>, Option<PageStorage>)>>,
}

impl PageStates for SignedIn {
    fn take(&self, tab: String) -> futures_util::future::BoxFuture<'static, Result<TakenState, String>> {
        let storage = self.storage.clone();
        Box::pin(async move {
            match tab.as_str() {
                "t1" => Ok(TakenState { url: "http://localhost:3000/app".into(), title: "App".into(), mobile: true, storage }),
                _ => Err("The tab is gone.".into()),
            }
        })
    }

    fn keep(&self, token: String, sites: Vec<String>, storage: Option<PageStorage>) {
        self.kept.lock().unwrap().push((token, sites, storage));
    }
}

fn storage(value: &str) -> PageStorage {
    PageStorage {
        origin: "http://localhost:3000".into(),
        local: vec![StorageItem { key: "session".into(), value: value.into() }],
        session: Vec::new(),
        databases: Vec::new(),
        skipped: Vec::new(),
    }
}

/// Page states move over the stream: the agent's tab's state comes back as
/// `state`, without its storage past the size a page state moves, a tab
/// that is gone fails, and the user's browser's is kept under a token. Well
/// under a second.
#[tokio::test]
async fn page_states_move_over_the_stream() {
    let directory = tempfile::tempdir().unwrap();
    let browser = Arc::new(SignedIn { storage: storage("signed-in"), kept: Default::default() });
    let mut relay = Relay::open_with(engine(&directory), browser.clone());
    relay.send(&PreviewRelayMessage::StateTake { id: 1, tab: "t1".into() });
    let Frame::Control(taken) = relay.next().await else { panic!("the page state") };
    assert_eq!(
        taken,
        PreviewEngineMessage::State {
            id: 1,
            url: "http://localhost:3000/app".into(),
            title: "App".into(),
            mobile: true,
            storage: Some(storage("signed-in")),
            too_large: false,
        }
    );
    relay.send(&PreviewRelayMessage::StateTake { id: 2, tab: "t9".into() });
    let Frame::Control(gone) = relay.next().await else { panic!("the failure") };
    assert_eq!(gone, PreviewEngineMessage::Failed { id: 2, reason: "The tab is gone.".into() });
    relay.send(&PreviewRelayMessage::StateKeep {
        id: 3,
        token: "kept-1".into(),
        sites: vec!["http://localhost:3000".into()],
        storage: Some(storage("mine")),
    });
    let Frame::Control(kept) = relay.next().await else { panic!("the acknowledgement") };
    assert_eq!(kept, PreviewEngineMessage::StateKept { id: 3 });
    assert_eq!(
        *browser.kept.lock().unwrap(),
        vec![("kept-1".to_owned(), vec!["http://localhost:3000".to_owned()], Some(storage("mine")))]
    );

    // A page whose storage is larger than a frame moves its cookies only.
    let large = Arc::new(SignedIn { storage: storage(&"x".repeat(MAX_STORAGE_BYTES)), kept: Default::default() });
    let mut relay = Relay::open_with(engine(&directory), large);
    relay.send(&PreviewRelayMessage::StateTake { id: 1, tab: "t1".into() });
    let Frame::Control(PreviewEngineMessage::State { storage, too_large, .. }) = relay.next().await else {
        panic!("the page state")
    };
    assert_eq!((storage, too_large), (None, true));
}
