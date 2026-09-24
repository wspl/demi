//! Runners and their devices (`runner.md` § Connection and identity,
//! `web-api.md` § Workspaces, devices, and attached hosts, § Device log):
//! pairing, the runner socket's protocol, one live connection per device,
//! revocation, the pipe routes' authentication, the device routes, and a
//! backend restart, with real runners.

use std::time::Duration;

use demi_host_remote::testing::{RunnerProcess, RunnerProcessOptions};
use demi_runner_protocol::values::DeviceToken;
use demi_runner_protocol::wire::{self, HelloErrorCode, HostIdentity, Inbound, Outbound, RunnerInfo};
use demi_web_api::devices::{ClaimedDevice, DeviceKind, DeviceLog};
use demi_web_api::error::ErrorCode;
use demi_web_api::files::Directory;
use futures_util::{SinkExt as _, StreamExt as _};
use reqwest::StatusCode;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use crate::support::{Harness, TestBackend, eventually, stored_token};

#[tokio::test(flavor = "multi_thread")]
async fn a_claimed_runner_reconnects_with_its_token_until_its_device_is_revoked() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut runner = RunnerProcess::start(
        &backend.url,
        RunnerProcessOptions {
            name: "laptop".into(),
            ..RunnerProcessOptions::default()
        },
    );
    let code = runner.pairing_code(0).await;
    let claim = |code: String| backend.post("/api/devices/claim", Some(&master), json!({ "code": code }));

    let wrong = claim("AAAA-BBBB".into()).await;
    assert_eq!(wrong.refusal(), (StatusCode::NOT_FOUND, ErrorCode::InvalidCode));
    // Entered messy, the code still names the runner.
    let claimed = claim(format!(" {} ", code.to_lowercase())).await;
    assert_eq!(claimed.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&claimed.body));
    let device = claimed.json::<ClaimedDevice>().device;
    assert_eq!((device.name.as_str(), device.kind), ("laptop", DeviceKind::User));
    assert!(device.online);
    assert_eq!(device.home.as_deref(), Some(runner.home()));
    // A code is single use.
    assert_eq!(claim(code).await.refusal(), (StatusCode::NOT_FOUND, ErrorCode::InvalidCode));
    let listed = backend.devices(&master).await;
    assert_eq!(listed.len(), 1);
    assert_eq!((&listed[0].id, listed[0].online), (&device.id, true));

    // A restarted runner presents the token it stored and is the same device,
    // online.
    stored_token(&runner).await;
    runner.stop().await;
    backend.until_online(&master, device.id.as_str(), false).await;
    runner.start_again();
    backend.until_online(&master, device.id.as_str(), true).await;
    assert_eq!(backend.devices(&master).await.len(), 1);

    // Revoked, the device is gone, and its runner hears why and stops.
    let revoked = backend.delete(&format!("/api/devices/{}", device.id), &master).await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    eventually("the revoked runner stops", || {
        let stopped = !runner.running();
        async move { stopped }
    })
    .await;
    assert!(runner.output().contains("revoked"), "{}", runner.output());
    assert!(backend.devices(&master).await.is_empty());
    let again = backend.delete(&format!("/api/devices/{}", device.id), &master).await;
    assert_eq!(again.refusal(), (StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound));
    backend.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_runners_code_changes_while_it_waits_and_claims_are_limited() {
    let mut harness = Harness::new();
    harness.runners.claim_lifetime = Duration::from_secs(1);
    harness.runners.claims_per_minute = 3;
    let (backend, master) = harness.start_set_up().await;
    let runner = RunnerProcess::start(&backend.url, RunnerProcessOptions::default());
    let first = runner.pairing_code(0).await;
    let second = runner.pairing_code(1).await;
    assert_ne!(first, second);
    let claim = |code: &str| backend.post("/api/devices/claim", Some(&master), json!({ "code": code }));

    // The expired code is dead; the one the runner printed last claims it.
    let claimed = claim(&second).await;
    assert_eq!(claimed.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&claimed.body));
    assert_eq!(claim(&first).await.refusal(), (StatusCode::NOT_FOUND, ErrorCode::InvalidCode));
    let device = claimed.json::<ClaimedDevice>().device;
    backend.until_online(&master, device.id.as_str(), true).await;

    // Three attempts a minute: the fourth is refused before any code is
    // looked at, and a body that names no code is no attempt.
    assert_eq!(claim("").await.refusal().1, ErrorCode::InvalidBody);
    assert_eq!(claim("NOPE-NOPE").await.refusal(), (StatusCode::NOT_FOUND, ErrorCode::InvalidCode));
    assert_eq!(claim("NOPE-NOPE").await.refusal(), (StatusCode::TOO_MANY_REQUESTS, ErrorCode::RateLimited));
    backend.close().await;
}

/// A runner's side of the socket, spoken by the test.
struct RawRunner(tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>);

impl RawRunner {
    async fn connect(backend: &TestBackend) -> Self {
        let (socket, _) = tokio_tungstenite::connect_async(backend.ws_url("/api/runner")).await.unwrap();
        Self(socket)
    }

    async fn send(&mut self, message: &Outbound) {
        let frame = wire::encode(message).unwrap().into_bytes();
        self.0.send(Message::Binary(frame.into())).await.unwrap();
    }

    /// The next message the backend sends; none once it closed the socket.
    async fn next(&mut self) -> Option<Inbound> {
        loop {
            match tokio::time::timeout(Duration::from_secs(10), self.0.next()).await.unwrap() {
                Some(Ok(Message::Binary(frame))) => return Some(wire::decode(&frame).unwrap()),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => return None,
                Some(Ok(other)) => panic!("the backend sent {other:?}"),
            }
        }
    }
}

fn hello(protocol: u32, token: Option<&str>, managed: Option<bool>) -> Outbound {
    Outbound::Hello {
        protocol,
        device_token: token.map(|token| DeviceToken::try_from(token.to_owned()).unwrap()),
        runner: RunnerInfo {
            name: "raw".into(),
            platform: "test".into(),
            version: "0".into(),
            native_target: None,
            identity: HostIdentity {
                uid: 1,
                gid: 1,
                hostname: "raw".into(),
                home_dir: "/home/raw".into(),
            },
            managed,
        },
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runner_that_breaks_the_protocol_or_names_no_device_is_refused() {
    let mut harness = Harness::new();
    harness.runners.hello_deadline = Duration::from_millis(300);
    let backend = harness.start().await;

    let mut text = RawRunner::connect(&backend).await;
    text.0.send(Message::Text("{\"type\":\"not-a-runner-frame\"}".into())).await.unwrap();
    assert_eq!(text.next().await, None);

    let mut garbage = RawRunner::connect(&backend).await;
    garbage.0.send(Message::Binary(vec![0xc1, 0x00].into())).await.unwrap();
    assert_eq!(garbage.next().await, None);

    let refused = [
        (hello(wire::VERSION, Some("not-a-real-token"), None), HelloErrorCode::UnknownDevice, "unknown device"),
        (hello(wire::VERSION + 1, None, None), HelloErrorCode::UnsupportedProtocol, "unsupported protocol"),
        (hello(wire::VERSION, None, Some(true)), HelloErrorCode::UnknownDevice, "never paired"),
    ];
    for (message, code, words) in refused {
        let mut runner = RawRunner::connect(&backend).await;
        runner.send(&message).await;
        match runner.next().await {
            Some(Inbound::HelloError { code: refused, reason }) => {
                assert_eq!(refused, code);
                assert!(reason.contains(words), "{reason}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert_eq!(runner.next().await, None);
    }

    // A connection that says nothing is closed after the hello deadline.
    let mut silent = RawRunner::connect(&backend).await;
    let started = tokio::time::Instant::now();
    assert_eq!(silent.next().await, None);
    assert!(started.elapsed() >= Duration::from_millis(250), "{:?}", started.elapsed());
    backend.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_holds_one_live_connection_and_a_newcomer_is_adopted_once_the_first_is_gone() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut first = backend.pair(&master, "laptop").await;
    let token = first.token().await;

    // The same token from a second process is refused while the first is
    // connected, and the first keeps its connection.
    let mut twin = RunnerProcess::start(
        &backend.url,
        RunnerProcessOptions {
            name: "laptop".into(),
            token: Some(token),
            ..RunnerProcessOptions::default()
        },
    );
    eventually("the twin is refused", || {
        let refused = twin.output().contains("already_connected");
        async move { refused }
    })
    .await;
    assert!(backend.online(&master, first.id()).await);
    assert!(twin.running());

    // Once the first is gone, the twin's next attempt is adopted.
    first.runner.stop().await;
    backend.until_online(&master, first.id(), true).await;
    assert!(twin.running());
    twin.stop().await;
    backend.until_online(&master, first.id(), false).await;
    backend.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hellos_with_one_token_at_once_bind_one_socket_and_a_repeated_hello_changes_nothing() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let token = laptop.token().await;
    laptop.runner.stop().await;
    backend.until_online(&master, laptop.id(), false).await;

    let (mut one, mut other) = tokio::join!(RawRunner::connect(&backend), RawRunner::connect(&backend));
    let message = hello(wire::VERSION, Some(&token), None);
    tokio::join!(one.send(&message), other.send(&message));
    let (first, second) = tokio::join!(one.next(), other.next());
    let (mut bound, refused) = match (first, second) {
        (Some(Inbound::HelloOk { .. }), refused) => (one, refused),
        (refused, Some(Inbound::HelloOk { .. })) => (other, refused),
        answers => panic!("expected one welcome, got {answers:?}"),
    };
    assert!(
        matches!(&refused, Some(Inbound::HelloError { code: HelloErrorCode::AlreadyConnected, .. })),
        "{refused:?}"
    );
    assert!(backend.online(&master, laptop.id()).await);
    // A second hello on the bound socket is no new registration: the
    // backend answers nothing, and the socket stays the device's.
    bound.send(&message).await;
    bound.send(&Outbound::Pong { jobs: 0 }).await;
    assert!(tokio::time::timeout(Duration::from_millis(300), bound.next()).await.is_err());
    assert!(backend.online(&master, laptop.id()).await);
    drop(bound);
    backend.until_online(&master, laptop.id(), false).await;
    backend.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claim_whose_runner_went_away_makes_no_device() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut runner = RawRunner::connect(&backend).await;
    runner.send(&hello(wire::VERSION, None, None)).await;
    let Some(Inbound::ClaimPending { claim_token }) = runner.next().await else {
        panic!("the runner was given no code");
    };
    runner.0.close(None).await.unwrap();
    drop(runner);
    let claimed = backend
        .post("/api/devices/claim", Some(&master), json!({ "code": claim_token }))
        .await;
    assert_eq!(claimed.refusal(), (StatusCode::NOT_FOUND, ErrorCode::InvalidCode));
    assert!(backend.devices(&master).await.is_empty());
    backend.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pipe_is_reached_only_with_a_device_token() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let laptop = backend.pair(&master, "laptop").await;
    let token = laptop.token().await;
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let pipe = format!("{}/api/pipes/0123456789abcdef", backend.url);

    for request in [http.get(&pipe), http.put(&pipe).body("bytes")] {
        let answer = request.try_clone().unwrap().send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(answer.text().await.unwrap(), "device token required");
        let bogus = request.try_clone().unwrap().bearer_auth("not-a-token").send().await.unwrap();
        assert_eq!(bogus.status(), StatusCode::UNAUTHORIZED);
        // The browser's session is not a device's credential.
        let cookie = request.try_clone().unwrap().header("cookie", &master.cookie).send().await.unwrap();
        assert_eq!(cookie.status(), StatusCode::UNAUTHORIZED);
        let unknown = request.bearer_auth(&token).send().await.unwrap();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    }
    backend.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paired_device_is_browsed_and_its_log_read_through_device_access() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let home = laptop.runner.home().to_owned();
    std::fs::write(laptop.runner.home_dir().join("hello.txt"), "hi").unwrap();

    let browsed = backend.get(&format!("/api/devices/{}/fs", laptop.id()), Some(&master)).await;
    assert_eq!(browsed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&browsed.body));
    let listing = browsed.json::<Directory>();
    assert_eq!(listing.path, home);
    assert_eq!(listing.home.as_deref(), Some(home.as_str()));
    let hello = listing.entries.iter().find(|entry| entry.name == "hello.txt").unwrap();
    assert_eq!((hello.is_directory, hello.is_symbolic_link, hello.size), (false, false, 2));
    let made = format!("{home}/made/by/web");
    let created = backend
        .post(&format!("/api/devices/{}/fs", laptop.id()), Some(&master), json!({ "path": made }))
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert!(laptop.runner.home_dir().join("made/by/web").is_dir());
    for refused in ["?path=relative", "?path="] {
        let answer = backend.get(&format!("/api/devices/{}/fs{refused}", laptop.id()), Some(&master)).await;
        assert_eq!(answer.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery), "{refused}");
    }
    let missing = backend
        .get(&format!("/api/devices/{}/fs?path={home}/nothing", laptop.id()), Some(&master))
        .await;
    assert_eq!(missing.refusal(), (StatusCode::NOT_FOUND, ErrorCode::FsError));

    // The log answers by cursor, limit and source; the runner writes a line
    // to its files a moment after the event, so the read is repeated until
    // it holds the line.
    let path = format!("/api/devices/{}/log", laptop.id());
    let mut tail = backend.get(&path, Some(&master)).await.json::<DeviceLog>();
    for _ in 0..500 {
        if tail.lines.iter().any(|line| line.text == "online") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        tail = backend.get(&path, Some(&master)).await.json::<DeviceLog>();
    }
    assert!(tail.lines.iter().any(|line| line.text == "online"), "{:?}", tail.lines);
    assert!(tail.lines.iter().any(|line| line.text == "waiting to be paired"));
    assert!(tail.lines.iter().all(|line| line.source == "runner"));
    let code = laptop.runner.pairing_code(0).await;
    // The pairing code is for the console alone.
    assert!(!tail.lines.iter().any(|line| line.text.contains(&code)));
    let after = backend.get(&format!("{path}?since={}", tail.next), Some(&master)).await;
    assert_eq!(after.json::<DeviceLog>(), DeviceLog { lines: Vec::new(), next: tail.next });
    let first = backend.get(&format!("{path}?since=0&limit=1"), Some(&master)).await.json::<DeviceLog>();
    assert_eq!(first.lines, tail.lines[..1]);
    let second = backend
        .get(&format!("{path}?since={}&limit=1", first.next), Some(&master))
        .await
        .json::<DeviceLog>();
    assert_eq!(second.lines, tail.lines[1..2]);
    let other = backend.get(&format!("{path}?since=0&source=service%3Anone"), Some(&master)).await;
    assert_eq!(other.json::<DeviceLog>(), DeviceLog { lines: Vec::new(), next: tail.next });
    for query in ["limit=0", "limit=1001", "limit=many", "since=-1", "since=1.5", "source="] {
        let refused = backend.get(&format!("{path}?{query}"), Some(&master)).await;
        assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery), "{query}");
    }
    let unknown = backend.get("/api/devices/none/log", Some(&master)).await;
    assert_eq!(unknown.refusal(), (StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound));

    // Offline, the device answers 409 instead of waking anything.
    laptop.runner.stop().await;
    backend.until_online(&master, laptop.id(), false).await;
    let offline = backend.get(&path, Some(&master)).await;
    assert_eq!(offline.refusal(), (StatusCode::CONFLICT, ErrorCode::DeviceOffline));
    let offline = backend.get(&format!("/api/devices/{}/fs", laptop.id()), Some(&master)).await;
    assert_eq!(offline.refusal(), (StatusCode::CONFLICT, ErrorCode::DeviceOffline));
    backend.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_backend_restart_the_devices_are_kept_and_their_runners_come_back() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let laptop = backend.pair(&master, "laptop").await;
    // Devices list oldest first, by the time they were paired.
    harness.clock.advance(jiff::SignedDuration::from_secs(1));
    let desktop = backend.pair(&master, "desktop").await;
    let address = backend.address();
    backend.close().await;

    // The session and the devices are records: both outlive the process.
    let backend = harness.start_at(address).await;
    for device in [&laptop, &desktop] {
        backend.until_online(&master, device.id(), true).await;
    }
    let devices = backend.devices(&master).await;
    let names: Vec<&str> = devices.iter().map(|device| device.name.as_str()).collect();
    assert_eq!(names, ["laptop", "desktop"]);
    assert!(devices.iter().all(|device| device.last_seen_at.is_some()));
    backend.close().await;
}
