//! Port forwards between the user's Hosts (`crates/backend-user-shard/src/shard/port_forwards.rs`), with
//! real runners: the Listen On runner listens on 127.0.0.1 only and
//! carries each connection's bytes both ways, over the relay or over a
//! direct peer between the two runners; a taken port says so until Try
//! Again finds it free; a connection to an offline target ends at once; a
//! restarted runner listens again; a relayed connection a target resets
//! ends both ways; a change of where a forward listens or connects ends its
//! connections, and a removed Host takes its forwards; a forward's state
//! comes only from its Listen On and a peer closes only from its pair; a
//! forward from a Host to itself joins there and counts Direct; and `demi
//! host forward add` and `remove` need Manage Port Forwards. The program at
//! the far end is a TCP echo server in the test process, which both runners
//! reach on 127.0.0.1.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use demi_runner_protocol::forward::ForwardRunState;
use demi_web_api_protocol::port_forwards::{PortForwardAnswer, PortForwardDto};
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use demi_runner_protocol::wire::{self, ArtifactOwner, Inbound, Outbound, StreamArtifactOwner};

use crate::conversations::{FIRST, Socket};
use crate::runners::{RawRunner, hello};
use crate::permissions::{categories, decide, permissions};
use crate::subagents::{Scripts, background, say, tree_on};
use crate::support::{Harness, Session, SyncChannel, TestBackend, pattern};

/// A TCP server on 127.0.0.1 that sends back what each connection sends,
/// and ends its writing when the connection's reading ends.
async fn echo() -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut reader, mut writer) = socket.split();
                // A connection the test cut short ends its copy; nothing waits for it.
                let _ = tokio::io::copy(&mut reader, &mut writer).await;
                let _ = writer.shutdown().await;
            });
        }
    });
    address
}

/// A port on 127.0.0.1 nothing listens on now.
fn free_port() -> u16 {
    std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The forward `id` once its state satisfies `done`, from the snapshot
/// and the `portForwards` parts the runners' announcements bring.
async fn until_forward(sync: &mut SyncChannel, id: &str, done: impl Fn(&PortForwardDto) -> bool) -> PortForwardDto {
    let waiting = async {
        loop {
            let forwards = match sync.next().await {
                SyncEvent::Snapshot { state } => state.port_forwards,
                SyncEvent::PortForwards { port_forwards } => port_forwards,
                _ => continue,
            };
            if let Some(forward) = forwards.into_iter().find(|forward| forward.id.as_str() == id)
                && done(&forward)
            {
                return forward;
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), waiting)
        .await
        .expect("the forward reaches the state within 30 s")
}

fn listening(direct: u32, relay: u32) -> impl Fn(&PortForwardDto) -> bool {
    move |forward| {
        matches!(forward.state, Some(ForwardRunState::Listening { direct: d, relay: r, .. }) if d == direct && r == relay)
    }
}

async fn create(backend: &TestBackend, session: &Session, body: serde_json::Value) -> PortForwardDto {
    let created = backend.post("/api/port-forwards", Some(session), body).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    created.json::<PortForwardAnswer>().forward
}

/// Sends `bytes` through the forward at `port` and answers what came back
/// before the far end ended.
async fn round_trip(port: u16, bytes: &[u8]) -> Vec<u8> {
    let program = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
    let (mut reader, mut writer) = program.into_split();
    let writing = async {
        writer.write_all(bytes).await.unwrap();
        writer.shutdown().await.unwrap();
    };
    let reading = async {
        let mut back = Vec::new();
        reader.read_to_end(&mut back).await.unwrap();
        back
    };
    tokio::join!(writing, reading).1
}

/// Whether a connection to the forward at `port` ends with no bytes, as a
/// refused one does: closed or reset.
async fn ends_at_once(port: u16) -> bool {
    let mut program = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
    let mut read = Vec::new();
    match program.read_to_end(&mut read).await {
        Ok(_) => read.is_empty(),
        Err(error) => error.kind() == std::io::ErrorKind::ConnectionReset,
    }
}

/// This computer's address on its network, not 127.0.0.1: the source
/// address of a route out, which a UDP socket picks without sending.
fn network_address() -> std::net::IpAddr {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    socket.connect(("192.0.2.1", 9)).expect("this computer has a route to a network");
    let address = socket.local_addr().unwrap().ip();
    assert!(!address.is_loopback() && !address.is_unspecified(), "{address}");
    address
}

// Several seconds: two real runners, one of them restarted.
#[tokio::test]
async fn a_forward_relays_both_ways_reports_a_taken_port_refuses_an_offline_target_and_listens_again_after_a_restart() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut listen = backend.pair(&master, "listen").await;
    let mut connect = backend.pair(&master, "connect").await;
    // Relay Only: the pair never talks directly.
    let route = backend
        .patch(&format!("/api/devices/{}", connect.id()), &master, json!({ "route": "server" }))
        .await;
    assert_eq!(route.status, StatusCode::OK);
    let target = echo().await;
    let port = free_port();
    let mut sync = backend.sync(&master).await;
    let forward = create(
        &backend,
        &master,
        json!({
            "name": "echo",
            "listenDevice": listen.id(),
            "listenPort": port,
            "connectDevice": connect.id(),
            "connectAddress": format!("127.0.0.1:{}", target.port()),
        }),
    )
    .await;
    let id = forward.id.as_str().to_owned();
    until_forward(&mut sync, &id, listening(0, 0)).await;

    // Both ways over the relay, counted while it is open.
    let mut program = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
    program.write_all(b"hello").await.unwrap();
    let mut hello = [0; 5];
    program.read_exact(&mut hello).await.unwrap();
    assert_eq!(&hello, b"hello");
    until_forward(&mut sync, &id, listening(0, 1)).await;
    let sent = pattern(2_000_000, 7);
    let (mut reader, mut writer) = program.into_split();
    let writing = async {
        writer.write_all(&sent).await.unwrap();
        writer.shutdown().await.unwrap();
    };
    let reading = async {
        let mut back = Vec::new();
        reader.read_to_end(&mut back).await.unwrap();
        back
    };
    let back = tokio::join!(writing, reading).1;
    assert!(back == sent, "the bytes came back as they were sent: {} of {}", back.len(), sent.len());
    until_forward(&mut sync, &id, listening(0, 0)).await;

    // Only programs on the Host connect: nothing listens on its network address.
    let outside = TcpStream::connect((network_address(), port)).await;
    assert!(outside.is_err(), "the listener takes no connection from the network");

    // A port another program holds says so until Try Again finds it free.
    let taken = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let taken_port = taken.local_addr().unwrap().port();
    let moved = backend
        .patch(&format!("/api/port-forwards/{id}"), &master, json!({ "listenPort": taken_port }))
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    until_forward(&mut sync, &id, |forward| forward.state == Some(ForwardRunState::PortInUse)).await;
    drop(taken);
    let retried = backend.post(&format!("/api/port-forwards/{id}/retry"), Some(&master), json!({})).await;
    assert_eq!(retried.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&retried.body));
    until_forward(&mut sync, &id, listening(0, 0)).await;
    assert_eq!(round_trip(taken_port, b"again").await, b"again");

    // An offline target: the connection ends at once, with nothing, and
    // the runner is not asked.
    connect.runner.stop().await;
    backend.until_online(&master, connect.id(), false).await;
    assert!(ends_at_once(taken_port).await, "the connection ends with no bytes");
    connect.runner.start_again();
    backend.until_online(&master, connect.id(), true).await;

    // A restarted Listen On runner listens again with no one asking.
    listen.runner.stop().await;
    until_forward(&mut sync, &id, |forward| forward.state.is_none()).await;
    listen.runner.start_again();
    until_forward(&mut sync, &id, listening(0, 0)).await;
    assert_eq!(round_trip(taken_port, b"back").await, b"back");

    // Removed, it stops listening.
    let removed = backend.delete(&format!("/api/port-forwards/{id}"), &master).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    sync.until(|event| matches!(event, SyncEvent::PortForwards { port_forwards } if port_forwards.is_empty()))
        .await;
    drop(listen);
}

// Several seconds: two real runners connect a peer of their own.
#[tokio::test]
async fn a_forward_goes_direct_once_the_two_runners_peer_connected() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let listen = backend.pair(&master, "listen").await;
    let connect = backend.pair(&master, "connect").await;
    let target = echo().await;
    let port = free_port();
    let mut sync = backend.sync(&master).await;
    let forward = create(
        &backend,
        &master,
        json!({
            "listenDevice": listen.id(),
            "listenPort": port,
            "connectDevice": connect.id(),
            "connectAddress": format!("127.0.0.1:{}", target.port()),
        }),
    )
    .await;
    let id = forward.id.as_str().to_owned();
    assert_eq!(forward.name, "Forward", "a forward without a name has the default");
    until_forward(&mut sync, &id, listening(0, 0)).await;

    // The first connection has the backend introduce the runners and takes
    // the relay, which nothing waits behind.
    let mut first = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
    first.write_all(b"1").await.unwrap();
    first.read_exact(&mut [0; 1]).await.unwrap();
    until_forward(&mut sync, &id, listening(0, 1)).await;

    // The peer's connection is announced to no one, so each next connection
    // is tried until one goes direct; each waits on its own count.
    let direct = loop {
        let mut next = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
        next.write_all(b"2").await.unwrap();
        next.read_exact(&mut [0; 1]).await.unwrap();
        let counted = until_forward(&mut sync, &id, |forward| {
            matches!(forward.state, Some(ForwardRunState::Listening { direct, relay, .. }) if direct + relay == 2)
        })
        .await;
        if matches!(counted.state, Some(ForwardRunState::Listening { direct: 1, .. })) {
            break next;
        }
        drop(next);
        until_forward(&mut sync, &id, listening(0, 1)).await;
    };
    let sent = pattern(2_000_000, 3);
    let (mut reader, mut writer) = direct.into_split();
    let writing = async {
        writer.write_all(&sent).await.unwrap();
        writer.shutdown().await.unwrap();
    };
    let reading = async {
        let mut back = vec![];
        reader.read_to_end(&mut back).await.unwrap();
        back
    };
    let back = tokio::join!(writing, reading).1;
    assert!(back == sent, "the bytes came back as they were sent: {} of {}", back.len(), sent.len());
    until_forward(&mut sync, &id, listening(0, 1)).await;
    drop(first);
    until_forward(&mut sync, &id, listening(0, 0)).await;
    drop((listen, connect));
}

// Several seconds: a real device runs a shell job that waits for the
// user's answer, and the turns its end and the decision wake.
#[tokio::test]
async fn adding_and_removing_a_forward_from_a_conversation_needs_manage_port_forwards() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, device, _, _provider) = tree_on(&scripts, Harness::new()).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let target = echo().await;
    let port = free_port();
    let add = format!(
        "demi host forward add --listen {id}:{port} --connect {id}:127.0.0.1:{} --name echo",
        target.port(),
        id = device.id()
    );

    // Listing needs no permission; the add waits for the user's answer,
    // and once allowed the rest of the script runs under the grant.
    scripts.root(
        FIRST,
        vec![
            background(
                "t1",
                &format!(
                    "demi host forward list; {add} && demi host forward list && demi host forward list --json && demi host forward remove echo"
                ),
            ),
            say("I asked the user"),
            say("done"),
            say("done"),
        ],
    );
    socket.chat("m1", "Forward the echo server").await;
    let ran = scripts.asked(|session| session == FIRST).pop().unwrap();
    // An empty list is its header row; listing needs no permission.
    assert!(ran.contains("output:\\nID  NAME  LISTEN  CONNECT  STATE\\n"), "listing needs no permission: {ran}");
    assert!(
        ran.contains("waiting for the user's permission to manage port forwards; its end will report to you, so do not run it again"),
        "{ran}"
    );
    let forwards = backend.get("/api/port-forwards", Some(&master)).await;
    assert_eq!(forwards.json::<serde_json::Value>()["forwards"], json!([]), "nothing was added");
    let asked = permissions(&backend, &master, FIRST).await;
    assert_eq!(asked.requests.len(), 1, "{asked:?}");
    let request = &asked.requests[0];
    assert_eq!(request.command(), add);
    assert_eq!(categories(request)[0].id, "host.forward");
    assert_eq!(categories(request)[0].action.as_deref(), Some("manage port forwards"));

    let allowed = decide(&backend, &master, FIRST, request.id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    let ran = crate::permissions::settled(
        &scripts,
        &backend,
        &master,
        FIRST,
        &["The user allowed this conversation to manage port forwards.", "(t1) ended with exit code 0"],
    )
    .await;
    // The new forward as labelled lines, then a row of the list and its
    // JSON, each Host by its name.
    let (id, name) = (device.id(), &device.device.name);
    let connect = format!("{name}:127.0.0.1:{}", target.port());
    let added = format!("Name: echo\\nListen: {name}:{port}\\nConnect: {connect}\\nEnabled: yes\\nState: ");
    assert!(ran.contains(&added), "{ran}");
    let forward = ran.split("\\nID: ").nth(1).and_then(|rest| rest.split("\\n").next()).expect(&ran);
    assert!(ran.contains("\\nID  NAME  LISTEN  "), "{ran}");
    assert!(ran.contains(&format!("\\n{forward}  echo  {name}:{port}  {connect}  ")), "{ran}");
    assert!(
        ran.contains(&format!(
            r#"[{{\"id\":\"{forward}\",\"name\":\"echo\",\"listenHost\":\"{name}\",\"listenDevice\":\"{id}\",\"listenPort\":{port},\"connectHost\":\"{name}\",\"connectDevice\":\"{id}\",\"connectAddress\":\"127.0.0.1:{}\",\"enabled\":true,\"state\":"#,
            target.port()
        )),
        "{ran}"
    );
    assert!(ran.contains("Removed echo."), "{ran}");
    let forwards = backend.get("/api/port-forwards", Some(&master)).await;
    assert_eq!(forwards.json::<serde_json::Value>()["forwards"], json!([]));
}

/// What a watched target says of its connections, each by the order it
/// accepted them in.
#[derive(Debug, PartialEq, Eq)]
enum Seen {
    Accepted(usize),
    Ended(usize),
}

/// An echo server on 127.0.0.1 that says when it accepts a connection and
/// when that connection ends: closed or reset by the runner.
async fn watched_echo() -> (SocketAddr, tokio::sync::mpsc::UnboundedReceiver<Seen>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (seen, watching) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut accepted = 0;
        while let Ok((mut socket, _)) = listener.accept().await {
            accepted += 1;
            let number = accepted;
            let seen = seen.clone();
            // The test may have stopped watching.
            let _ = seen.send(Seen::Accepted(number));
            tokio::spawn(async move {
                let (mut reader, mut writer) = socket.split();
                // A reset ends the copy as a close does.
                let _ = tokio::io::copy(&mut reader, &mut writer).await;
                let _ = writer.shutdown().await;
                let _ = seen.send(Seen::Ended(number));
            });
        }
    });
    (address, watching)
}

/// The next thing the target saw, which a broken connection never says.
async fn next_seen(seen: &mut tokio::sync::mpsc::UnboundedReceiver<Seen>) -> Seen {
    tokio::time::timeout(std::time::Duration::from_secs(30), seen.recv())
        .await
        .expect("the target hears of the connection")
        .expect("the target watches")
}

/// A program's connection through the forward at `port`, once the target
/// answered its first bytes.
async fn opened(port: u16) -> TcpStream {
    let mut program = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
    program.write_all(b"hi").await.unwrap();
    let mut answer = [0; 2];
    program.read_exact(&mut answer).await.unwrap();
    assert_eq!(&answer, b"hi");
    program
}

// Several seconds: two real runners.
#[tokio::test]
async fn a_relayed_connection_whose_target_resets_it_ends_both_ways_though_its_program_says_nothing() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let listen = backend.pair(&master, "listen").await;
    let connect = backend.pair(&master, "connect").await;
    let route = backend
        .patch(&format!("/api/devices/{}", connect.id()), &master, json!({ "route": "server" }))
        .await;
    assert_eq!(route.status, StatusCode::OK);
    // The target resets each connection once it read a byte.
    let resetting = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let target = resetting.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = resetting.accept().await {
            let mut byte = [0; 1];
            // A connection that went first has nothing to reset.
            let _ = socket.read_exact(&mut byte).await;
            #[allow(deprecated, reason = "a zero linger is how a test resets a connection")]
            let _ = socket.set_linger(Some(std::time::Duration::ZERO));
            drop(socket);
        }
    });
    let port = free_port();
    let mut sync = backend.sync(&master).await;
    let forward = create(
        &backend,
        &master,
        json!({
            "listenDevice": listen.id(),
            "listenPort": port,
            "connectDevice": connect.id(),
            "connectAddress": format!("127.0.0.1:{}", target.port()),
        }),
    )
    .await;
    let id = forward.id.as_str().to_owned();
    until_forward(&mut sync, &id, listening(0, 0)).await;

    // The program writes a byte and then nothing, its side left open.
    let mut program = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
    program.write_all(b"x").await.unwrap();
    until_forward(&mut sync, &id, listening(0, 1)).await;
    // The reset ends the connection both ways: the program hears its end,
    // and the forward counts it closed, though the program never ended
    // its side.
    let mut rest = Vec::new();
    let _ = program.read_to_end(&mut rest).await;
    until_forward(&mut sync, &id, listening(0, 0)).await;
    drop((listen, connect, program));
}

// Several seconds: two real runners carry five connections in turn.
#[tokio::test]
async fn changing_where_a_forward_listens_or_connects_ends_its_connections_and_a_removed_host_takes_its_forwards() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let listen = backend.pair(&master, "listen").await;
    let connect = backend.pair(&master, "connect").await;
    let route = backend
        .patch(&format!("/api/devices/{}", connect.id()), &master, json!({ "route": "server" }))
        .await;
    assert_eq!(route.status, StatusCode::OK);
    let (first_target, mut first) = watched_echo().await;
    let (second_target, mut second) = watched_echo().await;
    let port = free_port();
    let mut sync = backend.sync(&master).await;
    let forward = create(
        &backend,
        &master,
        json!({
            "name": "echo",
            "listenDevice": listen.id(),
            "listenPort": port,
            "connectDevice": connect.id(),
            "connectAddress": format!("127.0.0.1:{}", first_target.port()),
        }),
    )
    .await;
    let id = forward.id.as_str().to_owned();
    let path = format!("/api/port-forwards/{id}");
    until_forward(&mut sync, &id, listening(0, 0)).await;

    // Another port: the open connection ends, and the new port carries.
    let _program = opened(port).await;
    assert_eq!(next_seen(&mut first).await, Seen::Accepted(1));
    let moved_port = free_port();
    let moved = backend.patch(&path, &master, json!({ "listenPort": moved_port })).await;
    assert_eq!(moved.status, StatusCode::OK);
    assert_eq!(next_seen(&mut first).await, Seen::Ended(1));
    until_forward(&mut sync, &id, |forward| forward.listen_port == moved_port && listening(0, 0)(forward)).await;
    let mut program = opened(moved_port).await;
    assert_eq!(next_seen(&mut first).await, Seen::Accepted(2));

    // A new name changes nothing else: the connection still carries.
    let renamed = backend.patch(&path, &master, json!({ "name": "renamed" })).await;
    assert_eq!(renamed.status, StatusCode::OK);
    program.write_all(b"still").await.unwrap();
    let mut still = [0; 5];
    program.read_exact(&mut still).await.unwrap();
    assert_eq!(&still, b"still");

    // Another address: the open connection ends, new ones go there.
    let readdressed = backend
        .patch(&path, &master, json!({ "connectAddress": format!("127.0.0.1:{}", second_target.port()) }))
        .await;
    assert_eq!(readdressed.status, StatusCode::OK);
    assert_eq!(next_seen(&mut first).await, Seen::Ended(2));
    let _program = opened(moved_port).await;
    assert_eq!(next_seen(&mut second).await, Seen::Accepted(1));

    // Turned off, its connections end and it no longer listens. The page
    // hears that before it is turned on again: the states it heard before,
    // listening with no connection once the connection to the first target
    // ended, would otherwise answer the wait below before the runner
    // listens again.
    let off = backend.patch(&path, &master, json!({ "enabled": false })).await;
    assert_eq!(off.status, StatusCode::OK);
    assert_eq!(next_seen(&mut second).await, Seen::Ended(1));
    until_forward(&mut sync, &id, |forward| !forward.enabled && forward.state.is_none()).await;

    // Turned on again, it carries; its Connect To Host removed, the forward
    // goes with it, and so do its connections.
    let on = backend.patch(&path, &master, json!({ "enabled": true })).await;
    assert_eq!(on.status, StatusCode::OK);
    until_forward(&mut sync, &id, listening(0, 0)).await;
    let _program = opened(moved_port).await;
    assert_eq!(next_seen(&mut second).await, Seen::Accepted(2));
    let revoked = backend.delete(&format!("/api/devices/{}", connect.id()), &master).await;
    assert!(revoked.status.is_success(), "{}", String::from_utf8_lossy(&revoked.body));
    sync.until(|event| matches!(event, SyncEvent::PortForwards { port_forwards } if port_forwards.is_empty()))
        .await;
    assert_eq!(next_seen(&mut second).await, Seen::Ended(2));
    drop((listen, connect));
}

/// A runner's peers that carry nothing: the Connect To the test speaks for
/// answers an offer with them, and no channel is opened on them.
struct NoOperations;

impl demi_runner_direct::Operations for NoOperations {
    fn read(
        &self,
        _: demi_runner_direct::Scope,
        _: String,
        _: u64,
        _: Option<u64>,
    ) -> demi_runner_direct::Answer<demi_runner_direct::FileRange> {
        Box::pin(async { Err(refused()) })
    }
    fn write(&self, _: demi_runner_direct::Scope, _: String, _: bool, _: demi_runner_direct::ByteStream) -> demi_runner_direct::Answer<()> {
        Box::pin(async { Err(refused()) })
    }
    fn text(&self, _: demi_runner_direct::Scope, _: String, _: Option<String>) -> demi_runner_direct::Answer<demi_runner_direct::FileText> {
        Box::pin(async { Err(refused()) })
    }
    fn list(&self, _: demi_runner_direct::Scope, _: Option<String>) -> demi_runner_direct::Answer<demi_runner_direct::Listing> {
        Box::pin(async { Err(refused()) })
    }
    fn mkdir(&self, _: demi_runner_direct::Scope, _: String) -> demi_runner_direct::Answer<()> {
        Box::pin(async { Err(refused()) })
    }
    fn delete(&self, _: demi_runner_direct::Scope, _: String) -> demi_runner_direct::Answer<()> {
        Box::pin(async { Err(refused()) })
    }
    fn watch(
        &self,
        _: demi_runner_direct::Scope,
        _: tokio::sync::mpsc::UnboundedReceiver<demi_runner_protocol::files::FileWatchRequest>,
    ) -> demi_runner_direct::WatchStream {
        Box::pin(futures_util::stream::empty())
    }
    fn stream(&self, _: demi_runner_direct::Scope, _: demi_runner_direct::StreamRequest) -> demi_runner_direct::Answer<demi_runner_direct::ByteStream> {
        Box::pin(async { Err(refused()) })
    }
    fn stream_activity(&self, _: &str, name: &str) -> futures_util::future::BoxFuture<'static, demi_runner_direct::StreamActivity> {
        let stream = name.to_owned();
        Box::pin(async move { demi_runner_direct::StreamActivity { stream, held: Box::new(()) } })
    }
    fn forward(&self, _: String, _: String) -> demi_runner_direct::Answer<demi_runner_direct::Forwarded> {
        Box::pin(async { Err(refused()) })
    }
}

fn refused() -> demi_runner_protocol::direct::ChannelError {
    demi_runner_protocol::direct::ChannelError::new(
        demi_runner_protocol::direct::ChannelErrorCode::HostOperationFailed,
        404,
        "nothing here",
    )
}

/// A paired device whose runner the test speaks for, from its hello on.
async fn spoken_for(backend: &TestBackend, master: &Session, name: &str) -> (String, RawRunner) {
    let mut paired = backend.pair(master, name).await;
    let token = paired.token().await;
    paired.runner.stop().await;
    backend.until_online(master, paired.id(), false).await;
    let mut raw = RawRunner::connect(backend).await;
    raw.send(&hello(wire::VERSION, Some(&token), None)).await;
    loop {
        match raw.next().await {
            Some(Inbound::HelloOk { .. }) => break,
            Some(_) => {}
            None => panic!("the backend closed the socket"),
        }
    }
    backend.until_online(master, paired.id(), true).await;
    (paired.id().to_owned(), raw)
}

/// What the backend sends `raw` until it answered a request sent now,
/// which it answers after everything it was told before; pings answered.
async fn settled(raw: &mut RawRunner, id: &str) -> Vec<Inbound> {
    raw.send(&Outbound::ArtifactResolve {
        id: id.into(),
        owner: ArtifactOwner::Stream(StreamArtifactOwner {
            stream_id: "none".into(),
        }),
        sha256: "0".repeat(64),
        target: demi_command_protocol::host_target().into(),
    })
    .await;
    let mut heard = Vec::new();
    loop {
        match raw.next().await {
            Some(Inbound::ArtifactLocation { id: answered, .. }) if answered == id => return heard,
            Some(Inbound::Ping {}) => raw.send(&Outbound::Pong { jobs: 0 }).await,
            Some(other) => heard.push(other),
            None => panic!("the backend closed the socket"),
        }
    }
}

// Several seconds: a real runner offers a peer to a runner the test speaks
// for, and a third device the test speaks for says what is not its own.
#[tokio::test]
async fn a_forwards_state_comes_only_from_its_listen_on_and_a_peer_closes_only_from_its_pair() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let listen = backend.pair(&master, "listen").await;
    let (connect, mut connecting) = spoken_for(&backend, &master, "connect").await;
    let (_outsider, mut outsider) = spoken_for(&backend, &master, "outsider").await;
    let port = free_port();
    let mut sync = backend.sync(&master).await;
    let forward = create(
        &backend,
        &master,
        json!({
            "listenDevice": listen.id(),
            "listenPort": port,
            "connectDevice": connect,
            "connectAddress": "127.0.0.1:9",
        }),
    )
    .await;
    let id = forward.id.as_str().to_owned();
    until_forward(&mut sync, &id, listening(0, 0)).await;

    // Another device's word of the forward, as one the forward listened on
    // before says late, changes nothing.
    for (raw, request) in [(&mut connecting, "c1"), (&mut outsider, "o1")] {
        raw.send(&Outbound::ForwardState { forward: id.clone(), state: None }).await;
        settled(raw, request).await;
    }
    let listed = backend.get("/api/port-forwards", Some(&master)).await;
    let listed: demi_web_api_protocol::port_forwards::PortForwardList = listed.json();
    assert!(
        matches!(listed.forwards[0].state, Some(ForwardRunState::Listening { .. })),
        "{:?}",
        listed.forwards[0].state
    );

    // A connection has the listening runner offer a peer, which the
    // Connect To answers through peers of its own in the test.
    let (direct, driver) = demi_runner_direct::direct(
        std::sync::Arc::new(NoOperations),
        demi_runner_direct::Addresses::Only(vec![Ipv4Addr::LOCALHOST]),
    );
    tokio::spawn(driver.run());
    let _program = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
    let (request, peer, sdp) = loop {
        match connecting.next().await {
            Some(Inbound::DirectOffer { id, peer, sdp, .. }) => break (id, peer, sdp),
            Some(Inbound::Ping {}) => connecting.send(&Outbound::Pong { jobs: 0 }).await,
            Some(_) => {}
            None => panic!("the backend closed the socket"),
        }
    };
    let introduced = demi_runner_protocol::direct::Introduced::Runner { device: listen.id().to_owned() };
    let answered = direct.offer(peer.clone(), sdp, introduced, Vec::new()).await.expect("the peers answer");
    connecting.send(&Outbound::DirectAnswer { id: request, sdp: answered.sdp }).await;

    // The outsider's close is not its own: the pair hears nothing of it.
    outsider.send(&Outbound::DirectClosed { peer: peer.clone() }).await;
    settled(&mut outsider, "o2").await;
    let heard = settled(&mut connecting, "c2").await;
    assert!(
        !heard.iter().any(|message| matches!(message, Inbound::DirectClose { peer: closed } if *closed == peer)),
        "{heard:?}"
    );
    drop(listen);
}

// A second: one real runner joins its own connections.
#[tokio::test]
async fn a_forward_from_a_host_to_itself_joins_there_and_counts_direct() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let host = backend.pair(&master, "host").await;
    let target = echo().await;
    let port = free_port();
    let mut sync = backend.sync(&master).await;
    let forward = create(
        &backend,
        &master,
        json!({
            "listenDevice": host.id(),
            "listenPort": port,
            "connectDevice": host.id(),
            "connectAddress": format!("127.0.0.1:{}", target.port()),
        }),
    )
    .await;
    let id = forward.id.as_str().to_owned();
    until_forward(&mut sync, &id, listening(0, 0)).await;
    let program = opened(port).await;
    until_forward(&mut sync, &id, listening(1, 0)).await;
    drop(program);
    until_forward(&mut sync, &id, listening(0, 0)).await;
    drop(host);
}
