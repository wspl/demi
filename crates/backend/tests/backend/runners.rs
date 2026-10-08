//! Runners and their devices (`runner.md` § Connection and identity,
//! `web-api.md` § Workspaces, devices, and attached hosts, § Device log):
//! pairing, the runner socket's protocol, one live connection per device,
//! revocation, the pipe routes' authentication, the device routes, and a
//! backend restart, with real runners.

use std::time::Duration;

use demi_backend_remote_host::testing::{RunnerProcess, RunnerProcessOptions};
use demi_backend_user_shard::holds::HelloStep;
use demi_runner_process::backend::Backend;
use demi_command_protocol::ServiceSequence;
use demi_conversation_socket_protocol::ClientFrame;
use demi_runner_protocol::values::DeviceToken;
use demi_runner_protocol::wire::{
    self, ArtifactOwner, HelloErrorCode, HostIdentity, Inbound, Outbound, RunnerInfo,
    RunnerPlatform, StreamArtifactOwner,
};
use demi_web_api_protocol::devices::{DeviceAnswer, DeviceKind, DeviceLog, DeviceRoute, DeviceState};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::files::Directory;
use futures_util::{SinkExt as _, StreamExt as _};
use reqwest::StatusCode;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::conversations::{FIRST, Socket, create};
use crate::support::{Harness, Session, TestBackend, eventually, stored_token};

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
    let claim =
        |code: String| backend.post("/api/devices/claim", Some(&master), json!({ "code": code }));

    let wrong = claim("AAAA-BBBB".into()).await;
    assert_eq!(
        wrong.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::InvalidCode)
    );
    // Entered messy, the code still names the runner.
    let claimed = claim(format!(" {} ", code.to_lowercase())).await;
    assert_eq!(
        claimed.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&claimed.body)
    );
    let device = claimed.json::<DeviceAnswer>().device;
    assert_eq!(
        (device.name.as_str(), device.kind),
        ("laptop", DeviceKind::User)
    );
    assert_eq!(device.state, DeviceState::Online);
    assert_eq!(device.home.as_deref(), Some(runner.home()));
    // A code is single use.
    assert_eq!(
        claim(code).await.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::InvalidCode)
    );
    let listed = backend.devices(&master).await;
    assert_eq!(listed.len(), 1);
    assert_eq!((&listed[0].id, listed[0].state), (&device.id, DeviceState::Online));

    // A restarted runner presents the token it stored and is the same device,
    // online.
    stored_token(&runner).await;
    runner.stop().await;
    backend
        .until_online(&master, device.id.as_str(), false)
        .await;
    runner.start_again();
    backend
        .until_online(&master, device.id.as_str(), true)
        .await;
    assert_eq!(backend.devices(&master).await.len(), 1);

    // Revoked, the device is gone, and its runner hears why and removes
    // itself.
    let revoked = backend
        .delete(&format!("/api/devices/{}", device.id), &master)
        .await;
    assert_eq!(revoked.status, StatusCode::OK);
    eventually("the revoked runner stops", || {
        let stopped = !runner.running();
        async move { stopped }
    })
    .await;
    assert!(runner.output().contains("revoked"), "{}", runner.output());
    assert!(backend.devices(&master).await.is_empty());
    let again = backend
        .delete(&format!("/api/devices/{}", device.id), &master)
        .await;
    assert_eq!(
        again.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound)
    );
    backend.close().await;
}

// Over a second: a pairing code's lifetime of one second passes in real time.
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
    let claim =
        |code: &str| backend.post("/api/devices/claim", Some(&master), json!({ "code": code }));

    // The expired code is dead; the one the runner printed last claims it.
    let claimed = claim(&second).await;
    assert_eq!(
        claimed.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&claimed.body)
    );
    assert_eq!(
        claim(&first).await.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::InvalidCode)
    );
    let device = claimed.json::<DeviceAnswer>().device;
    backend
        .until_online(&master, device.id.as_str(), true)
        .await;

    // Three attempts a minute: the fourth is refused before any code is
    // looked at, and a body that names no code is no attempt.
    assert_eq!(claim("").await.refusal().1, ErrorCode::InvalidBody);
    assert_eq!(
        claim("NOPE-NOPE").await.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::InvalidCode)
    );
    assert_eq!(
        claim("NOPE-NOPE").await.refusal(),
        (StatusCode::TOO_MANY_REQUESTS, ErrorCode::RateLimited)
    );
    backend.close().await;
}

/// A runner's side of the socket, spoken by the test.
pub(crate) struct RawRunner(
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
);

impl RawRunner {
    pub(crate) async fn connect(backend: &TestBackend) -> Self {
        let (socket, _) = tokio_tungstenite::connect_async(backend.ws_url("/api/runner"))
            .await
            .unwrap();
        Self(socket)
    }

    pub(crate) async fn send(&mut self, message: &Outbound) {
        let frame = wire::encode(message).unwrap().into_bytes();
        self.0.send(Message::Binary(frame.into())).await.unwrap();
    }

    /// The projects a `revoked` names, once the backend sends it. What the
    /// backend sent the device before, such as its manifest, is passed over.
    async fn revoked(&mut self) -> Vec<String> {
        loop {
            match self.next().await {
                Some(Inbound::Revoked { projects }) => return projects,
                Some(_) => {}
                None => panic!("the connection closed without revoked"),
            }
        }
    }

    /// Answers the backend's next ping; what the backend sent the device
    /// before is passed over.
    async fn answer_ping(&mut self) {
        loop {
            match self.next().await {
                Some(Inbound::Ping {}) => break,
                Some(_) => {}
                None => panic!("the connection closed before a ping"),
            }
        }
        self.send(&Outbound::Pong { jobs: 0 }).await;
    }

    /// Reads until the backend closed the socket.
    async fn closed(&mut self) {
        while self.next().await.is_some() {}
    }

    /// The next message the backend sends; none once it closed the socket.
    pub(crate) async fn next(&mut self) -> Option<Inbound> {
        loop {
            match tokio::time::timeout(Duration::from_secs(10), self.0.next())
                .await
                .unwrap()
            {
                Some(Ok(Message::Binary(frame))) => return Some(wire::decode(&frame).unwrap()),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => return None,
                Some(Ok(other)) => panic!("the backend sent {other:?}"),
            }
        }
    }
}

pub(crate) fn hello(protocol: u32, token: Option<&str>, managed: Option<bool>) -> Outbound {
    Outbound::Hello {
        protocol,
        device_token: token.map(|token| DeviceToken::try_from(token.to_owned()).unwrap()),
        runner: RunnerInfo {
            name: "raw".into(),
            platform: RunnerPlatform::Linux,
            os: wire::OperatingSystem {
                name: "Ubuntu 26.04".into(),
                arch: "x86_64".into(),
            },
            version: "0".into(),
            native_target: None,
            identity: HostIdentity {
                uid: 1,
                gid: 1,
                hostname: "raw".into(),
                home_dir: "/home/raw".into(),
            },
            managed,
            installation: None,
        },
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runner_that_breaks_the_protocol_or_names_no_device_is_refused() {
    let mut harness = Harness::new();
    harness.runners.hello_deadline = Duration::from_millis(300);
    let backend = harness.start().await;

    let mut text = RawRunner::connect(&backend).await;
    text.0
        .send(Message::Text("{\"type\":\"not-a-runner-frame\"}".into()))
        .await
        .unwrap();
    assert_eq!(text.next().await, None);

    let mut garbage = RawRunner::connect(&backend).await;
    garbage
        .0
        .send(Message::Binary(vec![0xc1, 0x00].into()))
        .await
        .unwrap();
    assert_eq!(garbage.next().await, None);

    let refused = [
        (
            hello(wire::VERSION, Some("not-a-real-token"), None),
            HelloErrorCode::UnknownDevice,
            "unknown device",
        ),
        (
            hello(wire::VERSION + 1, None, None),
            HelloErrorCode::UnsupportedProtocol,
            "unsupported protocol",
        ),
        (
            hello(wire::VERSION, None, Some(true)),
            HelloErrorCode::UnknownDevice,
            "never paired",
        ),
    ];
    for (message, code, words) in refused {
        let mut runner = RawRunner::connect(&backend).await;
        runner.send(&message).await;
        match runner.next().await {
            Some(Inbound::HelloError {
                code: refused,
                reason,
            }) => {
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
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "{:?}",
        started.elapsed()
    );
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

/// Hellos with one token at once bind one socket: the other is a second
/// runner as long as the bound one answers its ping (`runner.md`
/// § Connection and identity).
#[tokio::test(flavor = "multi_thread")]
async fn hellos_with_one_token_at_once_bind_one_socket_and_a_repeated_hello_changes_nothing() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let token = laptop.token().await;
    laptop.runner.stop().await;
    backend.until_online(&master, laptop.id(), false).await;

    let (mut one, mut other) =
        tokio::join!(RawRunner::connect(&backend), RawRunner::connect(&backend));
    let message = hello(wire::VERSION, Some(&token), None);
    tokio::join!(one.send(&message), other.send(&message));
    // The waiting one hears nothing until the bound one answers its ping.
    let (mut bound, mut waiting) = tokio::select! {
        answer = one.next() => match answer {
            Some(Inbound::HelloOk { .. }) => (one, other),
            answer => panic!("expected a welcome, got {answer:?}"),
        },
        answer = other.next() => match answer {
            Some(Inbound::HelloOk { .. }) => (other, one),
            answer => panic!("expected a welcome, got {answer:?}"),
        },
    };
    bound.answer_ping().await;
    let refused = waiting.next().await;
    assert!(
        matches!(
            &refused,
            Some(Inbound::HelloError {
                code: HelloErrorCode::AlreadyConnected,
                ..
            })
        ),
        "{refused:?}"
    );
    assert!(backend.online(&master, laptop.id()).await);
    // A second hello on the bound socket is no new registration: the
    // backend answers nothing, and the socket stays the device's. The backend
    // handles a runner's messages in order, so the refusal of a request sent
    // after the hello is the first answer that arrives.
    bound.send(&message).await;
    bound.send(&Outbound::Pong { jobs: 0 }).await;
    let after = Outbound::ArtifactResolve {
        id: "after-hello".into(),
        owner: ArtifactOwner::Stream(StreamArtifactOwner {
            stream_id: "none".into(),
        }),
        sha256: "0".repeat(64),
        target: demi_command_protocol::host_target().into(),
    };
    bound.send(&after).await;
    match bound.next().await {
        Some(Inbound::ArtifactLocation {
            id, error: Some(_), ..
        }) if id == "after-hello" => {}
        answer => panic!("expected the refusal of the later request first, got {answer:?}"),
    }
    assert!(backend.online(&master, laptop.id()).await);
    drop(bound);
    backend.until_online(&master, laptop.id(), false).await;
    backend.close().await;
}

/// A connection lost without a close leaves the backend holding a socket
/// nobody answers; the runner's new hello finds it silent and takes the
/// device once the probe's wait is over (`runner.md` § Connection and
/// identity).
#[tokio::test(flavor = "multi_thread")]
async fn a_held_connection_that_does_not_answer_its_ping_gives_way_to_the_devices_new_hello() {
    let mut harness = Harness::new();
    let probe = Duration::from_millis(300);
    harness.runners.probe = probe;
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let token = laptop.token().await;
    laptop.runner.stop().await;
    backend.until_online(&master, laptop.id(), false).await;
    let message = hello(wire::VERSION, Some(&token), None);

    // The held socket reads nothing more after its welcome, as one whose
    // network dropped it.
    let mut held = RawRunner::connect(&backend).await;
    held.send(&message).await;
    assert!(matches!(held.next().await, Some(Inbound::HelloOk { .. })));

    let mut returning = RawRunner::connect(&backend).await;
    let asked = tokio::time::Instant::now();
    returning.send(&message).await;
    assert!(matches!(returning.next().await, Some(Inbound::HelloOk { .. })));
    assert!(asked.elapsed() >= probe, "{:?}", asked.elapsed());
    held.closed().await;
    assert!(backend.online(&master, laptop.id()).await);
    drop(returning);
    backend.until_online(&master, laptop.id(), false).await;
    backend.close().await;
}

/// Revoking a device tells its connected runner, which then removes itself
/// (`runner.md` § Installation, pairing and removal), before the backend
/// closes the connection.
#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_devices_runner_hears_it_was_revoked_before_its_connection_closes() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let token = laptop.token().await;
    laptop.runner.stop().await;
    backend.until_online(&master, laptop.id(), false).await;

    let mut runner = RawRunner::connect(&backend).await;
    runner.send(&hello(wire::VERSION, Some(&token), None)).await;
    assert!(matches!(runner.next().await, Some(Inbound::HelloOk { .. })));
    let revoked = backend
        .delete(&format!("/api/devices/{}", laptop.id()), &master)
        .await;
    assert_eq!(revoked.status, StatusCode::OK);
    assert_eq!(runner.revoked().await, Vec::<String>::new());
    assert_eq!(runner.next().await, None);
    backend.close().await;
}

/// A runner asks the backend to revoke its device, as `run uninstall` does
/// (`runner.md` § Installation, pairing and removal), which nothing refuses:
/// the device leaves the list with its attachment, and its connection ends
/// with `revoked`, which names the project that went with it.
#[tokio::test(flavor = "multi_thread")]
async fn a_runner_asks_for_its_devices_revocation_and_hears_which_projects_went() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let token = laptop.token().await;
    let home = laptop.runner.home_dir().to_str().unwrap().to_owned();
    let workspace = backend
        .post(
            "/api/workspaces",
            Some(&master),
            json!({ "kind": "device", "deviceId": laptop.id(), "path": home, "name": "proj" }),
        )
        .await;
    assert_eq!(workspace.status, StatusCode::CREATED);
    let conversation = uuid::Uuid::new_v4().to_string();
    create(&backend, &master, &conversation).await;
    let hosts = format!("/api/conversations/{conversation}/hosts");
    harness.attach(&conversation, laptop.id(), "laptop");
    laptop.runner.stop().await;
    backend.until_online(&master, laptop.id(), false).await;

    let mut runner = RawRunner::connect(&backend).await;
    runner.send(&hello(wire::VERSION, Some(&token), None)).await;
    assert!(matches!(runner.next().await, Some(Inbound::HelloOk { .. })));
    runner.send(&Outbound::Revoke {}).await;
    assert_eq!(runner.revoked().await, ["proj"]);
    assert_eq!(runner.next().await, None);
    assert!(backend.devices(&master).await.is_empty());
    let attached: serde_json::Value = backend.get(&hosts, Some(&master)).await.json();
    assert_eq!(attached["hosts"], json!([]));
    backend.close().await;
}

/// A native service's numbers come from its conversation's `tab` sequence,
/// each once, and only for a conversation of the device's user that reaches
/// the device (`native-runtime.md` § Conversation numbers).
#[tokio::test(flavor = "multi_thread")]
async fn a_runner_reserves_numbers_only_of_its_users_conversations_that_reach_it() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let token = laptop.token().await;
    let here = uuid::Uuid::new_v4().to_string();
    create(&backend, &master, &here).await;
    let home = laptop.runner.home_dir().to_str().unwrap().to_owned();
    let target = json!({ "target": { "kind": "device", "deviceId": laptop.id(), "path": home } });
    let moved = backend
        .patch(&format!("/api/conversations/{here}"), &master, target)
        .await;
    assert_eq!(
        moved.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&moved.body)
    );
    // A conversation on the Cloud does not reach the laptop.
    let elsewhere = uuid::Uuid::new_v4().to_string();
    create(&backend, &master, &elsewhere).await;
    laptop.runner.stop().await;
    backend.until_online(&master, laptop.id(), false).await;

    let mut runner = RawRunner::connect(&backend).await;
    runner.send(&hello(wire::VERSION, Some(&token), None)).await;
    assert!(matches!(runner.next().await, Some(Inbound::HelloOk { .. })));
    let mut reserve = async |conversation: &str, count: u32| {
        let id = uuid::Uuid::new_v4().to_string();
        runner
            .send(&Outbound::NumbersReserve {
                id: id.clone(),
                conversation_id: conversation.into(),
                sequence: ServiceSequence::Tab,
                count,
            })
            .await;
        loop {
            match runner.next().await {
                Some(Inbound::NumbersReserved {
                    id: answered,
                    first,
                    error,
                }) if answered == id => {
                    break match (first, error) {
                        (Some(first), None) => Ok(first),
                        (None, Some(error)) => Err(error),
                        answer => {
                            panic!("an answer carries its first number or its error: {answer:?}")
                        }
                    };
                }
                Some(_) => {}
                None => panic!("the backend closed the socket"),
            }
        }
    };
    assert_eq!(reserve(&here, 4).await, Ok(1));
    assert_eq!(reserve(&here, 1).await, Ok(5));
    assert!(reserve(&elsewhere, 1).await.is_err());
    assert!(reserve("no-such-conversation", 1).await.is_err());
    // A refused request takes no number.
    assert_eq!(reserve(&here, 2).await, Ok(6));
    drop(runner);
    backend.close().await;
}

/// A runner that goes away while its token is looked up is let go at once,
/// without waiting for the lookup: the backend never adopts its socket, so
/// the device never comes online for it (`runner.md` § Connection and
/// identity).
#[tokio::test(flavor = "multi_thread")]
async fn a_runner_that_goes_away_while_its_token_is_looked_up_is_let_go_and_never_comes_online() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let token = laptop.token().await;
    laptop.runner.stop().await;
    backend.until_online(&master, laptop.id(), false).await;

    let lookups = backend.hold_hellos(HelloStep::TokenLookup);
    let mut runner = RawRunner::connect(&backend).await;
    runner.send(&hello(wire::VERSION, Some(&token), None)).await;
    lookups.until_arrived(1).await;
    // The backend ends the connection while the lookup still waits.
    runner.0.close(None).await.unwrap();
    assert_eq!(runner.next().await, None);
    lookups.release();
    backend.close().await;
}

/// A runner whose hello reaches its device's shard as the backend shuts
/// down is not welcomed: a closing shard binds no runner, so the runner's
/// connection closes without an answer, and the shutdown ends (`runner.md`
/// § Connection and identity, `backend.md` § Startup and shutdown).
#[tokio::test(flavor = "multi_thread")]
async fn a_runner_whose_hello_meets_the_shutdown_is_never_welcomed() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    let token = laptop.token().await;
    laptop.runner.stop().await;
    backend.until_online(&master, laptop.id(), false).await;
    // A page's conversation socket, which the shard closes early in its own
    // close, before its runners' connections; the answer to a frame shows
    // that the shard serves it.
    create(&backend, &master, FIRST).await;
    let mut page = Socket::connect(&backend, &master, FIRST).await;
    page.send(&ClientFrame::Abort {}).await;
    page.frame().await;

    let binds = backend.hold_hellos(HelloStep::Bind);
    let mut runner = RawRunner::connect(&backend).await;
    runner.send(&hello(wire::VERSION, Some(&token), None)).await;
    binds.until_arrived(1).await;
    let answered = async move {
        assert_eq!(page.closed().await, Some(u16::from(CloseCode::Away)));
        binds.release();
        let answer = runner.next().await;
        // A welcomed runner would keep the shutdown waiting for it.
        drop(runner);
        answer
    };
    let ((), answer) = tokio::join!(backend.close(), answered);
    assert_eq!(
        answer, None,
        "the backend welcomed a runner while it shut down"
    );
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
        .post(
            "/api/devices/claim",
            Some(&master),
            json!({ "code": claim_token }),
        )
        .await;
    assert_eq!(
        claimed.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::InvalidCode)
    );
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
        let bogus = request
            .try_clone()
            .unwrap()
            .bearer_auth("not-a-token")
            .send()
            .await
            .unwrap();
        assert_eq!(bogus.status(), StatusCode::UNAUTHORIZED);
        // The user's session cookie is not a device's credential.
        let cookie = request
            .try_clone()
            .unwrap()
            .header("cookie", &master.cookie)
            .send()
            .await
            .unwrap();
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

    let browsed = backend
        .get(&format!("/api/devices/{}/fs", laptop.id()), Some(&master))
        .await;
    assert_eq!(
        browsed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&browsed.body)
    );
    let listing = browsed.json::<Directory>();
    assert_eq!(listing.path, home);
    assert_eq!(listing.home.as_deref(), Some(home.as_str()));
    let hello = listing
        .entries
        .iter()
        .find(|entry| entry.name == "hello.txt")
        .unwrap();
    assert_eq!(
        (hello.is_directory, hello.is_symbolic_link, hello.size),
        (false, false, 2)
    );
    let made = format!("{home}/made/by/web");
    let created = backend
        .post(
            &format!("/api/devices/{}/fs", laptop.id()),
            Some(&master),
            json!({ "path": made }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert!(laptop.runner.home_dir().join("made/by/web").is_dir());
    for refused in ["?path=relative", "?path="] {
        let answer = backend
            .get(
                &format!("/api/devices/{}/fs{refused}", laptop.id()),
                Some(&master),
            )
            .await;
        assert_eq!(
            answer.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery),
            "{refused}"
        );
    }
    let missing = backend
        .get(
            &format!("/api/devices/{}/fs?path={home}/nothing", laptop.id()),
            Some(&master),
        )
        .await;
    assert_eq!(
        missing.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::FsError)
    );

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
    assert!(
        tail.lines.iter().any(|line| line.text == "online"),
        "{:?}",
        tail.lines
    );
    assert!(
        tail.lines
            .iter()
            .any(|line| line.text == "waiting to be paired")
    );
    assert!(tail.lines.iter().all(|line| line.source == "runner"));
    let code = laptop.runner.pairing_code(0).await;
    // The pairing code is for the console alone.
    assert!(!tail.lines.iter().any(|line| line.text.contains(&code)));
    let after = backend
        .get(&format!("{path}?since={}", tail.next), Some(&master))
        .await;
    assert_eq!(
        after.json::<DeviceLog>(),
        DeviceLog {
            lines: Vec::new(),
            next: tail.next
        }
    );
    let first = backend
        .get(&format!("{path}?since=0&limit=1"), Some(&master))
        .await
        .json::<DeviceLog>();
    assert_eq!(first.lines, tail.lines[..1]);
    let second = backend
        .get(
            &format!("{path}?since={}&limit=1", first.next),
            Some(&master),
        )
        .await
        .json::<DeviceLog>();
    assert_eq!(second.lines, tail.lines[1..2]);
    let other = backend
        .get(
            &format!("{path}?since=0&source=service%3Anone"),
            Some(&master),
        )
        .await;
    assert_eq!(
        other.json::<DeviceLog>(),
        DeviceLog {
            lines: Vec::new(),
            next: tail.next
        }
    );
    for query in [
        "limit=0",
        "limit=1001",
        "limit=many",
        "since=-1",
        "since=1.5",
        "source=",
    ] {
        let refused = backend.get(&format!("{path}?{query}"), Some(&master)).await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery),
            "{query}"
        );
    }
    let unknown = backend.get("/api/devices/none/log", Some(&master)).await;
    assert_eq!(
        unknown.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound)
    );

    // Offline, the device answers 409 instead of waking anything.
    laptop.runner.stop().await;
    backend.until_online(&master, laptop.id(), false).await;
    let offline = backend.get(&path, Some(&master)).await;
    assert_eq!(
        offline.refusal(),
        (StatusCode::CONFLICT, ErrorCode::DeviceOffline)
    );
    let offline = backend
        .get(&format!("/api/devices/{}/fs", laptop.id()), Some(&master))
        .await;
    assert_eq!(
        offline.refusal(),
        (StatusCode::CONFLICT, ErrorCode::DeviceOffline)
    );
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
    // The backend comes back at its address, where the runners reconnect.
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

/// A device shows the operating system and the runner release its runner's
/// hello named, the Cloud's too; a paired device takes a new name, trimmed,
/// which the user's pages see at once, or a route,
/// while the Cloud keeps its name and its path and another user's device is
/// not the caller's to change (`web-api.md` § Workspaces, devices, and
/// attached hosts).
// About half a second: a runner pairs and the Cloud boots.
#[tokio::test(flavor = "multi_thread")]
async fn a_device_shows_its_system_and_runner_release_and_a_paired_one_takes_a_new_name_or_route() {
    use demi_web_api_protocol::auth::Role;
    use demi_web_api_protocol::state::SyncEvent;

    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    harness.add_user("ana@example.test", "ana-pass-1", Role::User);
    let ana = backend.login("ana@example.test", "ana-pass-1").await;
    let laptop = backend.pair(&master, "laptop").await;
    // The conversation's first file listing boots the Cloud, whose runner
    // says hello as a paired one does.
    create(&backend, &master, FIRST).await;
    let listed = backend
        .get(&format!("/api/conversations/{FIRST}/fs"), Some(&master))
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&listed.body));

    let devices = backend.devices(&master).await;
    let paired = &devices[0];
    let os = paired.os.as_ref().expect("the hello named the system");
    assert!(!os.name.is_empty());
    assert_eq!(os.arch, std::env::consts::ARCH);
    assert_eq!(paired.runner_version.as_deref(), Some(demi_shared_artifacts::WORKSPACE_VERSION));
    let mut page = backend.sync(&master).await;
    let state = page.snapshot().await;
    let cloud = state
        .devices
        .iter()
        .find(|device| device.kind == DeviceKind::Managed)
        .expect("the Cloud booted");
    assert!(cloud.os.is_some(), "{cloud:?}");
    assert_eq!(cloud.runner_version.as_deref(), Some(demi_shared_artifacts::WORKSPACE_VERSION));

    let rename = async |id: &str, session: &Session, name: &str| {
        backend
            .patch(&format!("/api/devices/{id}"), session, json!({ "name": name }))
            .await
    };
    let renamed = rename(laptop.id(), &master, "  Studio Mac  ").await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&renamed.body));
    assert_eq!(renamed.json::<DeviceAnswer>().device.name, "Studio Mac");
    page.until(|event| {
        matches!(event, SyncEvent::Devices { devices }
            if devices.iter().any(|device| device.name == "Studio Mac"))
    })
    .await;
    assert_eq!(backend.devices(&master).await[0].name, "Studio Mac");

    // 1 to 64 characters, counted as characters.
    for refused in ["   ".to_owned(), "x".repeat(65)] {
        assert_eq!(
            rename(laptop.id(), &master, &refused).await.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)
        );
    }
    let longest = "é".repeat(64);
    let renamed = rename(laptop.id(), &master, &longest).await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&renamed.body));
    assert_eq!(renamed.json::<DeviceAnswer>().device.name, longest);

    assert_eq!(
        rename(cloud.id.as_str(), &master, "Mine").await.refusal(),
        (StatusCode::CONFLICT, ErrorCode::DeviceManaged)
    );
    assert_eq!(
        rename(laptop.id(), &ana, "Mine").await.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound)
    );
    assert_eq!(backend.devices(&master).await[0].name, longest);

    // A new device's route is Automatic; Relay Only keeps its name, and
    // every page hears it. The Cloud has no route.
    assert_eq!(backend.devices(&master).await[0].route, DeviceRoute::Automatic);
    let server = backend
        .patch(&format!("/api/devices/{}", laptop.id()), &master, json!({ "route": "server" }))
        .await;
    assert_eq!(server.status, StatusCode::OK, "{}", String::from_utf8_lossy(&server.body));
    let device = server.json::<DeviceAnswer>().device;
    assert_eq!((device.name.as_str(), device.route), (longest.as_str(), DeviceRoute::Server));
    page.until(|event| {
        matches!(event, SyncEvent::Devices { devices }
            if devices.iter().any(|device| device.kind == DeviceKind::User && device.route == DeviceRoute::Server))
    })
    .await;
    assert_eq!(
        backend
            .patch(&format!("/api/devices/{}", laptop.id()), &master, json!({ "route": "fastest" }))
            .await
            .refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)
    );
    assert_eq!(
        backend
            .patch(&format!("/api/devices/{}", cloud.id), &master, json!({ "route": "direct" }))
            .await
            .refusal(),
        (StatusCode::CONFLICT, ErrorCode::DeviceManaged)
    );
    backend.close().await;
}

/// A Cloud's runner reaches its backend through the runner socket the
/// machine manager mounts into the Cloud (`managed-hosts.md` § Backend
/// socket), with the backend's public URL as the origin: the socket answers
/// the runner's socket and pipes, and nothing a browser uses.
#[tokio::test(flavor = "multi_thread")]
async fn the_runner_socket_serves_runners_and_nothing_a_browser_uses() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runners.sock");
    let mut harness = Harness::new();
    harness.runner_socket = Some(path.clone());
    let backend = harness.start().await;
    let through = Backend::through(backend.url.parse().unwrap(), path);

    let (mut socket, _) = through
        .websocket(
            backend.ws_url("/api/runner"),
            tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default(),
        )
        .await
        .unwrap();
    let hello = wire::encode(&hello(wire::VERSION, None, None)).unwrap();
    socket
        .send(Message::Binary(hello.into_bytes().into()))
        .await
        .unwrap();
    let answer = loop {
        if let Message::Binary(frame) = socket.next().await.unwrap().unwrap() {
            break wire::decode::<Inbound>(&frame).unwrap();
        }
    };
    assert!(matches!(answer, Inbound::ClaimPending { .. }), "{answer:?}");

    let http = through.http(reqwest::Client::builder()).build().unwrap();
    let origin = through.origin().unwrap();
    let pipe = http.get(origin.join("/api/pipes/unknown").unwrap()).send().await.unwrap();
    assert_eq!(pipe.status(), StatusCode::UNAUTHORIZED);
    let setup = http.get(origin.join("/api/setup").unwrap()).send().await.unwrap();
    assert_eq!(setup.status(), StatusCode::NOT_FOUND);
    // The network still serves the web app.
    assert_eq!(backend.get("/api/setup", None).await.status, StatusCode::OK);
    backend.close().await;
}
