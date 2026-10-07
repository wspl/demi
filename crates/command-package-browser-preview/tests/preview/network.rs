//! Where the engine's requests may go (`preview.md` § Local network) and how
//! it sends them (§ Upstream requests).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use demi_command_package_browser_protocol::preview::PreviewMode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::task::AbortOnDropHandle;

use crate::support::{Relay, Site, embedded, engine, opening, request, top};

#[tokio::test]
async fn a_page_cannot_reach_a_more_private_network_than_its_own() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let public = top(&site.origin("www.site.test"));
    let fetch = |url: String| request(&url, PreviewMode::NoCors, "image", Some(public.clone()));
    for private in [
        site.https("127.0.0.1", "/echo"),
        site.https("localhost", "/echo"),
        site.https("local.test", "/echo"),
    ] {
        let refused = relay.fetch(public.clone(), fetch(private.clone())).await;
        assert_eq!(refused.unwrap_err(), "refused: local-network", "{private}");
    }
    // Refused before it was sent: no request reached the device.
    assert!(site.received().is_empty(), "{:?}", site.received());
    // A navigation the page starts is a request of the page.
    let navigation = request(&site.https("local.test", "/echo"), PreviewMode::Navigate, "iframe", Some(public.clone()));
    let frame = embedded(&site.origin("local.test"), &public.top, true);
    assert!(relay.fetch(frame.clone(), navigation.clone()).await.is_err());
    // One whose initiator the browser does not name goes no further than
    // its preview tab's top site: a public site's tab reaches no device, a
    // local development page's reaches the Host's loopback.
    let unnamed = |url: String| request(&url, PreviewMode::Navigate, "iframe", None);
    assert!(relay.fetch(frame, unnamed(site.https("local.test", "/echo"))).await.is_err());
    let development = top(&site.origin("localhost"));
    let nested = embedded(&site.origin("127.0.0.1"), &development.top, true);
    relay.fetch(nested, unnamed(site.https("127.0.0.1", "/echo"))).await.unwrap();
    // A development page reaches its Host's other servers, and a public one
    // the public network.
    relay
        .fetch(
            development.clone(),
            request(&site.https("127.0.0.1", "/echo"), PreviewMode::NoCors, "image", Some(development.clone())),
        )
        .await
        .unwrap();
    relay.fetch(public.clone(), fetch(site.https("other.test", "/echo"))).await.unwrap();
    // An address the user opens is not limited.
    relay.fetch(top(&site.origin("local.test")), opening(&site.https("local.test", "/echo"))).await.unwrap();
}

/// A server that closes the first connection when its second request
/// arrives, without an answer, as a server ending an idle connection the
/// client reuses at that moment does; every later connection answers each
/// request with its path.
struct Closing {
    port: u16,
    connections: Arc<AtomicUsize>,
    _serving: AbortOnDropHandle<()>,
}

impl Closing {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = connections.clone();
        let serving = tokio::spawn(async move {
            let mut served = tokio::task::JoinSet::new();
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let index = counted.fetch_add(1, Ordering::SeqCst);
                served.spawn(async move {
                    let mut answered = 0;
                    loop {
                        let mut head = Vec::new();
                        let mut byte = [0];
                        while !head.ends_with(b"\r\n\r\n") {
                            if stream.read(&mut byte).await.unwrap_or(0) == 0 {
                                return;
                            }
                            head.push(byte[0]);
                        }
                        if index == 0 && answered == 1 {
                            return;
                        }
                        let path = String::from_utf8_lossy(&head).split(' ').nth(1).unwrap().to_owned();
                        let answer = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\n\r\n{path}",
                            path.len()
                        );
                        stream.write_all(answer.as_bytes()).await.unwrap();
                        answered += 1;
                    }
                });
            }
        });
        Self {
            port,
            connections,
            _serving: AbortOnDropHandle::new(serving),
        }
    }
}

#[tokio::test]
async fn a_request_whose_reused_connection_closes_is_sent_again() {
    let directory = tempfile::tempdir().unwrap();
    let server = Closing::start().await;
    let mut relay = Relay::open(engine(&directory));
    let origin = format!("http://127.0.0.1:{}", server.port);
    let page = top(&origin);
    let first = relay.fetch(page.clone(), opening(&format!("{origin}/first"))).await.unwrap();
    assert_eq!(first.text(), "/first");
    let second = relay
        .fetch(page.clone(), request(&format!("{origin}/second"), PreviewMode::NoCors, "image", Some(page)))
        .await
        .unwrap();
    assert_eq!(second.text(), "/second");
    assert_eq!(server.connections.load(Ordering::SeqCst), 2);
}
