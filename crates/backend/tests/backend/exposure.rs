//! A device's exposed ports (`port-exposure.md`), with a real runner: a
//! Compatible entry whose endpoint names the user's device by its id
//! reaches a scripted vendor on the device's 127.0.0.1 once the port is
//! exposed, and streams its answer; before, the request is refused without
//! reaching the device, and the failed turn and Test Connection offer to
//! expose the port; a stopped turn and a removed port each end the runner's
//! connection to the port in the middle of an answer; an entry naming
//! another user's device, or a redirect naming one of the user's, reaches
//! an unknown host; and an offline device refuses at once, a failed turn
//! naming it. The vendor runs in the test process, which the runner reaches
//! on 127.0.0.1.

use std::time::Duration;

use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::{Block, NotExposedPort, OfflineDevice};
use demi_web_api_protocol::exposed_ports::{ExposedPortAnswer, ExposedPortDto};
use demi_web_api_protocol::providers::TestResult;
use reqwest::StatusCode;
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use crate::conversations::{FIRST, Socket, answer, choose, create, entry, send, transcript};
use crate::support::{Harness, Session, TestBackend};

/// A Compatible Messages API entry of `session`'s at `http://<device>:<port>/v1`.
async fn device_entry(backend: &TestBackend, session: &Session, device: &str, port: u16) -> String {
    let body = json!({
        "source": "compatible", "providerType": "anthropic", "label": "Studio models", "apiKey": "sk-ant-test",
        "baseUrl": format!("http://{device}:{port}/v1"),
        "models": [{
            "id": "model-a", "displayName": "A", "contextWindow": 200000, "outputLimit": 8000,
            "thinkingEfforts": [], "acceptedExtensions": [], "fastTier": null
        }]
    });
    entry(backend, session, body).await
}

async fn tested(backend: &TestBackend, session: &Session, provider: &str) -> TestResult {
    backend
        .post(&format!("/api/providers/{provider}/test"), Some(session), json!({ "modelId": "model-a" }))
        .await
        .json::<TestResult>()
}

async fn expose(backend: &TestBackend, session: &Session, device: &str, port: u16) -> ExposedPortDto {
    let created = backend
        .post("/api/exposed-ports", Some(session), json!({ "deviceId": device, "port": port }))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    created.json::<ExposedPortAnswer>().port
}

/// The most a wait on the runner takes when it does right.
const BUDGET: Duration = Duration::from_secs(10);

/// A model on a port of the device that begins its answer, the head and
/// the stream's first event, and then thinks for as long as it is let.
struct Thinking {
    listener: TcpListener,
    port: u16,
}

impl Thinking {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        Self { listener, port }
    }

    /// Takes the runner's request and begins the answer: the connection.
    async fn begin(&self) -> TcpStream {
        let (mut connection, _) = within(self.listener.accept()).await.unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            within(connection.read_exact(&mut byte)).await.unwrap();
            head.push(byte[0]);
        }
        let event = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"model-a\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n";
        let answer = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n{:x}\r\n{event}\r\n",
            event.len()
        );
        connection.write_all(answer.as_bytes()).await.unwrap();
        connection
    }
}

/// `future`, which must end within the budget.
async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(BUDGET, future).await.expect("the runner did this within the budget")
}

/// Waits until the runner closes `connection`.
async fn closed(mut connection: TcpStream) {
    let mut rest = Vec::new();
    within(connection.read_to_end(&mut rest)).await.unwrap();
}

/// The port the vendor listens on, on 127.0.0.1.
fn port_of(vendor: &MockVendor) -> u16 {
    reqwest::Url::parse(&vendor.url("/")).unwrap().port().unwrap()
}

// A backend, a real runner and a scripted vendor: a few seconds.
#[tokio::test]
async fn a_compatible_entry_reaches_an_exposed_port_of_its_owners_device_and_streams_and_offers_to_expose_one_that_is_not() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let studio = backend.pair(&master, "Studio").await;
    let port = port_of(&vendor);
    let provider = device_entry(&backend, &master, studio.id(), port).await;
    let unexposed = NotExposedPort {
        device_id: studio.id().to_owned(),
        device: "Studio".into(),
        port,
    };

    // Not exposed: refused before anything reaches the device's network,
    // with the port to expose.
    assert_eq!(
        tested(&backend, &master, &provider).await,
        TestResult::Failed {
            message: format!("Port {port} of Studio isn’t exposed."),
            model: Some("A".into()),
            not_exposed: Some(unexposed.clone()),
        }
    );
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "model-a").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    socket.chat("m1", "Hello").await;
    let failed = transcript(&backend, &master, FIRST).await;
    let Some(Block::Error(error)) = failed.blocks.last() else {
        panic!("{:?}", failed.blocks);
    };
    assert_eq!(
        (error.code.as_deref(), error.message.as_str()),
        (Some("port_not_exposed"), format!("Port {port} of Studio isn’t exposed.").as_str())
    );
    let facts = failed.failures.as_ref().and_then(|failures| failures.get(&error.id));
    assert_eq!(facts.and_then(|facts| facts.not_exposed.clone()), Some(unexposed.clone()));
    assert!(vendor.requests().is_empty(), "nothing reached the vendor");

    // Exposed: the request goes through the runner to 127.0.0.1, headers
    // and body as sent, and the answer streams: the test passes on its
    // first event while the vendor holds the stream open.
    let exposed = expose(&backend, &master, studio.id(), port).await;
    // Exposed meanwhile from another tab, Expose Port on the failure finds
    // the port exposed: that is done, the same port, not a refusal.
    let again = backend
        .post("/api/exposed-ports", Some(&master), json!({ "deviceId": studio.id(), "port": port }))
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", String::from_utf8_lossy(&again.body));
    assert_eq!(again.json::<ExposedPortAnswer>().port.id, exposed.id);
    // The failure keeps naming the port, which the page now holds exposed
    // and so offers Try Again for.
    let failed = transcript(&backend, &master, FIRST).await;
    let facts = failed.failures.as_ref().and_then(|failures| failures.get(&error.id));
    assert_eq!(facts.and_then(|facts| facts.not_exposed.clone()), Some(unexposed));
    vendor.respond(answer(&["ok"], 3, 1).stay_open());
    assert_eq!(
        tested(&backend, &master, &provider).await,
        TestResult::Passed { model: "A".into() }
    );
    let sent = &vendor.requests()[0];
    assert_eq!(
        (sent.uri.path(), sent.header("x-api-key"), sent.json()["model"].as_str()),
        ("/v1/messages", Some("sk-ant-test"), Some("model-a"))
    );
    backend.close().await;
}

// A backend, a real runner and a model that thinks: a few seconds.
#[tokio::test]
async fn a_stopped_turn_and_a_removed_port_each_end_the_runners_connection_in_the_middle_of_an_answer() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let studio = backend.pair(&master, "Studio").await;
    let model = Thinking::new().await;
    let exposed = expose(&backend, &master, studio.id(), model.port).await;
    let provider = device_entry(&backend, &master, studio.id(), model.port).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "model-a").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // Stopped while the model thinks: its connection closes.
    socket.send(&send("m1", "Hello")).await;
    let connection = model.begin().await;
    socket.stop().await;
    closed(connection).await;

    // The port removed while the model thinks: its connection closes, and
    // the turn ends without it.
    socket.send(&send("m2", "Hello again")).await;
    let connection = model.begin().await;
    let removed = backend.delete(&format!("/api/exposed-ports/{}", exposed.id), &master).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&removed.body));
    closed(connection).await;
    socket.until_idle().await;
    let ended = transcript(&backend, &master, FIRST).await;
    let Some(Block::Error(error)) = ended.blocks.last() else {
        panic!("{:?}", ended.blocks);
    };
    assert_eq!(error.code.as_deref(), Some("port_not_exposed"), "{}", error.message);
    backend.close().await;
}

// A backend, three real runners, one of them stopped, two users and two
// scripted vendors, and an offline turn's retries with their backoff: up
// to ten seconds.
#[tokio::test]
async fn another_users_device_is_an_unknown_host_and_an_offline_device_refuses_at_once() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let port = port_of(&vendor);

    // The master's entry naming another user's device, which that user
    // exposed, goes to the network, where no host has that name; nothing
    // reaches the device.
    let other = backend.make_user(&master, "other@example.test", "user").await;
    let laptop = backend.pair(&other, "Laptop").await;
    expose(&backend, &other, laptop.id(), port).await;
    let theirs = device_entry(&backend, &master, laptop.id(), port).await;
    let TestResult::Failed { message, not_exposed, .. } = tested(&backend, &master, &theirs).await else {
        panic!("a host no network knows answers nothing");
    };
    assert!(!message.contains("exposed") && !message.contains("offline"), "{message}");
    assert_eq!(not_exposed, None);
    assert!(vendor.requests().is_empty(), "nothing reached the vendor");

    // A vendor on the network that redirects to the master's own device,
    // which exposed the port: the redirect is followed on the network,
    // where no host has that name, and nothing reaches the device.
    let desk = backend.pair(&master, "Desk").await;
    let model = Thinking::new().await;
    expose(&backend, &master, desk.id(), model.port).await;
    let network = MockVendor::start().await;
    let redirected = entry(&backend, &master, json!({
        "source": "compatible", "providerType": "anthropic", "label": "Gateway", "apiKey": "sk-ant-test",
        "baseUrl": network.url("/v1"),
        "models": [{
            "id": "model-a", "displayName": "A", "contextWindow": 200000, "outputLimit": 8000,
            "thinkingEfforts": [], "acceptedExtensions": [], "fastTier": null
        }]
    }))
    .await;
    network.respond(
        MockResponse::status(307).header("location", &format!("http://{}:{}/v1/messages", desk.id(), model.port)),
    );
    let TestResult::Failed { not_exposed, .. } = tested(&backend, &master, &redirected).await else {
        panic!("a host no network knows answers nothing");
    };
    assert_eq!(not_exposed, None);
    assert_eq!(network.requests().len(), 1, "the redirect was answered");
    let reached = tokio::time::timeout(Duration::ZERO, model.listener.accept()).await;
    assert!(reached.is_err(), "nothing reached the device");

    // The device offline: refused from the device's state, the runner not
    // asked.
    let mut studio = backend.pair(&master, "Studio").await;
    expose(&backend, &master, studio.id(), port).await;
    let ours = device_entry(&backend, &master, studio.id(), port).await;
    studio.runner.stop().await;
    backend.until_online(&master, studio.id(), false).await;
    assert_eq!(
        tested(&backend, &master, &ours).await,
        TestResult::Failed {
            message: "Studio is offline.".into(),
            model: Some("A".into()),
            not_exposed: None,
        }
    );
    // A turn fails naming the device, after the retries an unreachable
    // endpoint gets.
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &ours, "model-a").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    socket.chat("m1", "Hello").await;
    let failed = transcript(&backend, &master, FIRST).await;
    let Some(Block::Error(error)) = failed.blocks.last() else {
        panic!("{:?}", failed.blocks);
    };
    assert_eq!(error.code.as_deref(), Some("device_offline"), "{}", error.message);
    let facts = failed.failures.as_ref().and_then(|failures| failures.get(&error.id));
    assert_eq!(
        facts.and_then(|facts| facts.device_offline.clone()),
        Some(OfflineDevice {
            device_id: studio.id().to_owned(),
            device: "Studio".into(),
        })
    );
    assert!(vendor.requests().is_empty(), "nothing reached the vendor");
    backend.close().await;
}
