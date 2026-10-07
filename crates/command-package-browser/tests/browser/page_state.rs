//! Page states between the two browsers (`preview.md` § Page state), through
//! the program as the relay and the plugin reach it: `state_take` and
//! `state_keep` on the conversation's `preview` stream, and
//! `browser.handover`; and `present` (`preview.md` § Presenting a page),
//! which records the page for its command's card. They start the pinned
//! Chrome for Testing and are ignored unless asked for, as the other Chrome
//! tests are. A test takes a browser start and a few page loads, about 3 s.

use std::time::Duration;

use axum::Router;
use axum::http::HeaderMap;
use axum::response::{AppendHeaders, Html, IntoResponse};
use axum::routing::get;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use demi_command_package_browser_protocol::preview::{
    CHUNK_FRAME, CONTROL_FRAME, PageStorage, PreviewClient, PreviewCredentials, PreviewEngineMessage,
    PreviewEnvironment, PreviewMode, PreviewRelayMessage, PreviewRequest, PreviewScheme, StorageItem,
};
use demi_command_protocol::{CommandCaller, Record};
use demi_command_sdk::{Input, ServiceError};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::families::{BrowserFixture, with_browser_fixture};

/// A site whose page records, in its first script, what it finds of its
/// origin's storage and cookies, and what its server received of them;
/// with `?write=<who>` it writes its own,
/// as a page the agent signed in to does. `/set` sets an `HttpOnly` cookie
/// and a visible one; `/cookie` answers the `Cookie` header it received.
const PAGE: &str = r#"<!doctype html><title>State</title><script>
window.stateAtLoad = {
  local: localStorage.getItem('probe'),
  session: sessionStorage.getItem('probe'),
  cookie: document.cookie,
};
window.idbAtLoad = new Promise(resolve => {
  const open = indexedDB.open('probe-db');
  open.onupgradeneeded = () => open.transaction.abort();
  open.onerror = () => resolve(null);
  open.onsuccess = () => {
    const request = open.result.transaction('items').objectStore('items').get('k');
    request.onsuccess = () => { resolve(request.result); open.result.close(); };
  };
}).then(value => { window.idbRead = value ?? null; });
fetch('/cookie').then(answer => answer.text()).then(text => { window.serverCookie = text; });
const who = new URLSearchParams(location.search).get('write');
window.written = !who;
if (who) (async () => {
  await fetch('/set');
  localStorage.setItem('probe', `from-${who}`);
  sessionStorage.setItem('probe', `from-${who}`);
  await new Promise(resolve => {
    const open = indexedDB.open('probe-db', 1);
    open.onupgradeneeded = () => open.result.createObjectStore('items');
    open.onsuccess = () => {
      const transaction = open.result.transaction('items', 'readwrite');
      transaction.objectStore('items').put({ when: new Date(5), bytes: new Uint8Array([1, 2]) }, 'k');
      transaction.oncomplete = () => { open.result.close(); resolve(); };
    };
  });
  window.written = true;
})();
</script>"#;

struct Site {
    origin: String,
    _server: AbortOnDropHandle<()>,
}

async fn site() -> Site {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://localhost:{}", listener.local_addr().unwrap().port());
    let routes = Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route(
            "/set",
            get(|| async {
                (
                    AppendHeaders([
                        ("set-cookie", "signed=in; Path=/; HttpOnly"),
                        ("set-cookie", "visible=yes; Path=/"),
                    ]),
                    "set",
                )
                    .into_response()
            }),
        )
        .route(
            "/cookie",
            get(|headers: HeaderMap| async move {
                headers.get("cookie").and_then(|value| value.to_str().ok()).unwrap_or_default().to_owned()
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
    Site {
        origin,
        _server: AbortOnDropHandle::new(server),
    }
}

/// The conversation's `preview` stream, as the relay opens it.
struct Preview {
    input: mpsc::UnboundedSender<Result<Bytes, ServiceError>>,
    records: mpsc::Receiver<Record>,
    pending: BytesMut,
    _served: AbortOnDropHandle<Result<demi_command_protocol::Completion, ServiceError>>,
}

impl Preview {
    fn open(fixture: &BrowserFixture) -> Self {
        let (input, received) = mpsc::unbounded_channel();
        let (records, served) = fixture.start(
            "browser.preview",
            json!({}),
            CommandCaller::User {},
            Input::from_stream(futures_util::stream::unfold(received, |mut received| async move {
                received.recv().await.map(|item| (item, received))
            })),
            CancellationToken::new(),
        );
        let preview = Self {
            input,
            records,
            pending: BytesMut::new(),
            _served: AbortOnDropHandle::new(tokio::spawn(served)),
        };
        preview.send(&PreviewRelayMessage::Hello {
            scheme: PreviewScheme::Https,
            domain: "preview.test".into(),
            namespace: "k3f9a2ab".into(),
            host: "host-1".into(),
        });
        preview
    }

    fn send(&self, message: &PreviewRelayMessage) {
        let json = serde_json::to_vec(message).unwrap();
        let mut bytes = BytesMut::new();
        bytes.put_u32(1 + json.len() as u32);
        bytes.put_u8(CONTROL_FRAME);
        bytes.put_slice(&json);
        self.input.send(Ok(bytes.freeze())).unwrap();
    }

    async fn frame(&mut self) -> (u8, Bytes) {
        loop {
            if self.pending.len() >= 4 {
                let length = u32::from_be_bytes(self.pending[..4].try_into().unwrap()) as usize;
                if self.pending.len() >= 4 + length {
                    self.pending.advance(4);
                    let mut frame = self.pending.split_to(length).freeze();
                    return (frame.get_u8(), frame);
                }
            }
            let record = tokio::time::timeout(Duration::from_secs(30), self.records.recv())
                .await
                .expect("the engine's next frame")
                .expect("an open stream");
            let Record::Stdout(bytes) = record else {
                panic!("the stream writes only its bytes");
            };
            self.pending.extend_from_slice(&bytes);
        }
    }

    async fn message(&mut self) -> PreviewEngineMessage {
        let (kind, payload) = self.frame().await;
        assert_eq!(kind, CONTROL_FRAME);
        PreviewEngineMessage::decode(&payload).unwrap()
    }

    /// The body of a GET of `url` the engine makes with the jar, as the
    /// site's own page in the user's browser would.
    async fn get(&mut self, id: u32, url: &str) -> String {
        let environment = PreviewEnvironment {
            origin: url::Url::parse(url).unwrap().origin().ascii_serialization(),
            top: "http://localhost".into(),
            cross: false,
        };
        self.send(&PreviewRelayMessage::Request {
            id,
            environment: environment.clone(),
            request: PreviewRequest {
                url: url.into(),
                method: "GET".into(),
                headers: Vec::new(),
                body: false,
                mode: PreviewMode::Cors,
                destination: String::new(),
                credentials: PreviewCredentials::Include,
                referrer: String::new(),
                referrer_policy: String::new(),
                keepalive: false,
                // The site's own page asks.
                initiator: Some(environment),
                user: false,
            },
            client: PreviewClient {
                user_agent: "Mozilla/5.0 Chrome/154.0.0.0 Safari/537.36".into(),
                brands: String::new(),
                mobile: false,
                platform: "macOS".into(),
                accept_language: "en-US".into(),
            },
        });
        assert!(matches!(self.message().await, PreviewEngineMessage::Response { .. }));
        let mut body = Vec::new();
        loop {
            self.send(&PreviewRelayMessage::Pull { id });
            let (kind, chunk) = self.frame().await;
            assert_eq!(kind, CHUNK_FRAME);
            if chunk.len() == 4 {
                return String::from_utf8(body).unwrap();
            }
            body.extend_from_slice(&chunk[4..]);
        }
    }
}

/// `expression`'s value in `tab` once it is no longer false, as the page's
/// own asynchronous work settles.
async fn settled(fixture: &BrowserFixture, tab: &str, expression: &str) -> Value {
    for _ in 0..100 {
        // A page between documents answers no evaluation yet.
        let (code, answer) = fixture.result("browser.eval", json!({"tab": tab, "expression": expression}), CancellationToken::new()).await;
        let value = answer["value"].clone();
        if code == 0 && value != json!(false) && !value.is_null() {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{expression} never settled");
}

/// Open in Your Browser takes the agent's page: its `HttpOnly` cookie goes
/// into the jar, which the next request of the user's browser carries, and
/// its storage, IndexedDB's values with their types, comes back for the
/// state frame to write.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn the_agents_page_moves_into_the_users_browser_with_its_state() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let tab = fixture
            .call("browser.open", json!({"url": format!("{}/?write=agent", site.origin), "timeout": 120000}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        settled(&fixture, &tab, "window.written === true").await;
        let mut preview = Preview::open(&fixture);
        // Before: the jar knows nothing of the site.
        assert_eq!(preview.get(1, &format!("{}/cookie", site.origin)).await, "");
        preview.send(&PreviewRelayMessage::StateTake { id: 2, tab: tab.clone() });
        let PreviewEngineMessage::State { id: 2, url, title, storage: Some(storage), too_large: false } = preview.message().await else {
            panic!("the agent's page state");
        };
        assert_eq!((url, title), (format!("{}/?write=agent", site.origin), "State".to_owned()));
        assert_eq!(storage.origin, site.origin);
        assert_eq!(storage.local, vec![StorageItem { key: "probe".into(), value: "from-agent".into() }]);
        assert_eq!(storage.session, vec![StorageItem { key: "probe".into(), value: "from-agent".into() }]);
        let database = &storage.databases[0];
        assert_eq!(database["name"], "probe-db");
        assert_eq!(
            database["stores"][0]["records"][0],
            json!(["k", {"$": "object", "v": {
                "when": {"$": "date", "v": 5},
                "bytes": {"$": "typed", "type": "Uint8Array", "v": "AQI="},
            }}])
        );
        let cookies = preview.get(3, &format!("{}/cookie", site.origin)).await;
        assert!(cookies.contains("signed=in") && cookies.contains("visible=yes"), "{cookies}");
        // A tab that is gone fails, and the user's tab opens on its address alone.
        preview.send(&PreviewRelayMessage::StateTake { id: 4, tab: "t99".into() });
        assert!(matches!(preview.message().await, PreviewEngineMessage::Failed { id: 4, .. }));
        fixture
    })
    .await;
}

/// Open in Agent's Browser: the jar's cookies of the page's sites and the
/// storage the user's browser handed over are in place before the page's
/// first script, an `HttpOnly` cookie reaching the server but not
/// `document.cookie`.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn the_users_page_opens_in_the_agents_browser_with_its_state_before_its_first_script() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let mut preview = Preview::open(&fixture);
        // The user's browser signed in: the site set its cookies into the jar.
        assert_eq!(preview.get(1, &format!("{}/set", site.origin)).await, "set");
        let item = StorageItem { key: "probe".into(), value: "from-user".into() };
        let storage = PageStorage {
            origin: site.origin.clone(),
            local: vec![item.clone()],
            session: vec![item],
            databases: vec![json!({
                "name": "probe-db",
                "version": 1,
                "stores": [{
                    "name": "items", "keyPath": null, "autoIncrement": false, "indexes": [],
                    "records": [["k", "from-user-idb"]],
                }],
            })],
            skipped: Vec::new(),
        };
        preview.send(&PreviewRelayMessage::StateKeep { id: 2, sites: vec![site.origin.clone()], storage: Some(storage) });
        let PreviewEngineMessage::StateKept { id: 2, token } = preview.message().await else {
            panic!("the kept state's token");
        };
        let (code, opened) = fixture
            .result_for(
                CommandCaller::User {},
                "browser.handover",
                json!({"url": format!("{}/", site.origin), "state": token}),
                CancellationToken::new(),
                Vec::new(),
            )
            .await;
        assert_eq!(code, 0, "{opened}");
        let tab = opened["tab"].as_str().unwrap().to_owned();
        let at_load = settled(&fixture, &tab, "window.stateAtLoad").await;
        assert_eq!(at_load, json!({"local": "from-user", "session": "from-user", "cookie": "visible=yes"}));
        assert_eq!(settled(&fixture, &tab, "window.idbRead").await, json!("from-user-idb"));
        let server = settled(&fixture, &tab, "window.serverCookie").await;
        assert!(server.as_str().unwrap().contains("signed=in"), "{server}");
        fixture
    })
    .await;
}

/// `present` prints the page and records it in the job's record, which the
/// runner reads for the command's card.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn presenting_a_page_records_it_for_the_commands_card() {
    let record = tempfile::tempdir().unwrap();
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let mut fixture = fixture;
        fixture.edits = Some(demi_command_protocol::EditContext {
            directory: record.path().join("edits").to_string_lossy().into_owned(),
            lock: record.path().join("edits.lock").to_string_lossy().into_owned(),
        });
        let tab = fixture
            .call("browser.open", json!({"url": format!("{}/", site.origin), "timeout": 120000}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let presented = fixture.call("browser.present", json!({"tab": tab})).await;
        assert_eq!(presented, json!({"tab": tab, "title": "State", "url": format!("{}/", site.origin)}));
        let journal = demi_command_sdk::edits::Recorder::new(fixture.edits.clone().unwrap()).unwrap().report().unwrap();
        assert_eq!(
            journal.presented,
            vec![demi_command_protocol::PresentedPage { tab: tab.clone(), title: "State".into(), url: format!("{}/", site.origin) }]
        );
        fixture
    })
    .await;
}
