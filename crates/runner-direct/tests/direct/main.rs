//! The runner's peers (`direct-channel.md`), driven by a page in process
//! over loopback: making the channel, each operation's protocol, the
//! limits, closing, giving up, and the addresses offered. Each test runs a
//! real connection on 127.0.0.1 in tens of milliseconds; the give-up and
//! heartbeat tests pause the clock.

mod operations;

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

use bytes::Bytes;
use demi_command_protocol::{CommandLocale, PackageArtifact, PackageDescriptor};
use demi_runner_direct::{Addresses, Direct, direct, offered};
use demi_runner_protocol::direct::{
    Introduction, MAX_CHANNELS, MAX_PEERS, OfferRefusal, ServiceBinding, StunUrl,
};
use tokio::net::UdpSocket;
use tokio_util::task::AbortOnDropHandle;
use demi_runner_protocol::files::{FileWatchMessage, FileWatchState};
use if_addrs::{IfAddr, IfOperStatus, Ifv4Addr, Interface};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use operations::Fake;
use demi_runner_direct::testing::{Heard, Page};

fn introduction() -> Introduction {
    let package = PackageDescriptor {
        id: "demi.browser".into(),
        version: "1.0.0".into(),
        protocol_version: demi_command_protocol::VERSION,
        operations: vec!["live_view".into()],
        targets: BTreeMap::from([(
            "aarch64-apple-darwin".into(),
            PackageArtifact {
                sha256: "a".repeat(64),
                size: 1,
            },
        )]),
    };
    Introduction {
        color_scheme: demi_command_protocol::ColorScheme::Light,
        streams: BTreeMap::from([(
            "browser".into(),
            ServiceBinding {
                package,
                operation: "live_view".into(),
            },
        )]),
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en".into()],
        },
    }
}

/// The runner's peers on 127.0.0.1, driven on this test's runtime.
fn runner(fake: &Fake) -> Direct {
    let (direct, driver) = direct(
        Arc::new(fake.clone()),
        Addresses::Only(vec![Ipv4Addr::LOCALHOST]),
    );
    tokio::spawn(driver.run());
    direct
}

async fn connected(direct: &Direct, peer: &str) -> Page {
    let mut page = Page::offer(direct, peer, introduction())
        .await
        .expect("the runner answers");
    assert!(page.wait_connected().await, "the peer connects");
    page
}

/// A header for `op` in conversation `c1`, whose directory is `/work`.
fn header(op: &str, fields: Value) -> Value {
    let mut header = json!({ "op": op, "conversation": "c1", "cwd": "/work" });
    header
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    header
}

#[tokio::test]
async fn a_read_answers_the_range_and_refuses_a_version_the_file_no_longer_has() {
    let fake = Fake::default();
    let file: Vec<u8> = (0..300_000u32).map(|index| index as u8).collect();
    fake.put("/work/video.mp4", &file);
    let direct = runner(&fake);
    let page = connected(&direct, "p1").await;

    let mut whole = page.open(header("read", json!({ "path": "/work/video.mp4" }))).await;
    let answer = whole.next().await.json();
    assert_eq!(
        answer,
        json!({ "ok": true, "size": 300_000, "version": "W/\"1\"", "modifiedAt": "2026-09-21T14:13:20.123Z" })
    );
    assert_eq!(whole.bytes_to_end().await, file, "the whole file, then the channel's end");

    let mut range = page
        .open(header("read", json!({ "path": "/work/video.mp4", "offset": 1000, "length": 5, "version": "W/\"1\"" })))
        .await;
    assert_eq!(range.next().await.json()["ok"], true);
    assert_eq!(range.bytes_to_end().await, file[1000..1005]);

    fake.put("/work/video.mp4", b"another version");
    let mut stale = page
        .open(header("read", json!({ "path": "/work/video.mp4", "version": "W/\"1\"" })))
        .await;
    assert_eq!(stale.next().await.json()["error"]["code"], "file_changed");
    assert_eq!(stale.next().await, Heard::Closed, "no bytes of another version");
    assert_eq!(fake.held.lock().unwrap().scopes[0].cwd, "/work");
}

#[tokio::test]
async fn a_write_is_in_place_at_its_end_and_not_at_all_when_the_page_goes_first() {
    let fake = Fake::default();
    let direct = runner(&fake);
    let page = connected(&direct, "p1").await;

    let mut write = page
        .open(header("write", json!({ "path": "upload.bin", "replace": false })))
        .await;
    write.binary(b"first ");
    write.binary(b"second");
    write.text(r#"{"end":true}"#);
    assert_eq!(write.next().await.json(), json!({ "ok": true }));
    assert_eq!(write.next().await, Heard::Closed);
    assert_eq!(fake.file("upload.bin").as_deref(), Some(&b"first second"[..]));

    let cut = page
        .open(header("write", json!({ "path": "cut.bin", "replace": true })))
        .await;
    cut.binary(b"half");
    // The runner carries out the write before the page goes.
    fake.started_writes.subscribe().wait_for(|started| *started == 2).await.unwrap();
    cut.close();
    // The runner hears the close: the write ends without its file.
    fake.cut_writes.subscribe().wait_for(|cut| *cut == 1).await.unwrap();
    let mut taken = page
        .open(header("write", json!({ "path": "upload.bin", "replace": false })))
        .await;
    taken.text(r#"{"end":true}"#);
    assert_eq!(taken.next().await.json()["error"]["code"], "file_exists");
    assert_eq!(fake.file("cut.bin"), None, "a write cut short leaves nothing");
}

#[tokio::test]
async fn text_lists_folders_and_deletes_answer_as_the_relay() {
    let fake = Fake::default();
    fake.put("/work/notes.md", "héllo".as_bytes());
    let direct = runner(&fake);
    let page = connected(&direct, "p1").await;

    let mut text = page.open(header("text", json!({ "path": "/work/notes.md" }))).await;
    assert_eq!(
        text.next().await.json(),
        json!({ "ok": true, "version": "W/\"1\"", "unchanged": false })
    );
    assert_eq!(text.bytes_to_end().await, "héllo".as_bytes());
    let mut unchanged = page
        .open(header("text", json!({ "path": "/work/notes.md", "version": "W/\"1\"" })))
        .await;
    assert_eq!(unchanged.next().await.json()["unchanged"], true);
    assert_eq!(unchanged.next().await, Heard::Closed, "no text for a version the page holds");

    let mut list = page.open(header("list", json!({}))).await;
    let listed = list.next().await.json();
    assert_eq!(listed["path"], "/work", "the conversation's directory without a path");
    assert_eq!(listed["home"], "/home/ana");
    assert_eq!(listed["entries"][0]["name"], "notes.md");
    assert_eq!(listed["entries"][0]["size"], 6);

    let mut made = page.open(header("mkdir", json!({ "path": "/work/new" }))).await;
    assert_eq!(made.next().await.json(), json!({ "ok": true }));
    let mut deleted = page.open(header("delete", json!({ "path": "/work/notes.md" }))).await;
    assert_eq!(deleted.next().await.json(), json!({ "ok": true }));
    assert_eq!(fake.file("/work/notes.md"), None);

    let mut missing = page.open(header("text", json!({ "path": "/work/gone.md" }))).await;
    let refused = missing.next().await.json();
    assert_eq!(refused["error"]["code"], "fs_error");
    assert_eq!(refused["error"]["status"], 404);
    let mut invalid = page.open(json!({ "op": "read", "conversation": "c1" })).await;
    assert_eq!(invalid.next().await.json()["error"]["code"], "invalid_message");
}

#[tokio::test]
async fn a_stream_carries_bytes_both_ways_and_an_unknown_name_is_refused() {
    let fake = Fake::default();
    let direct = runner(&fake);
    let page = connected(&direct, "p1").await;

    let mut stream = page.open(header("stream", json!({ "stream": "browser" }))).await;
    assert_eq!(stream.next().await.json(), json!({ "ok": true }));
    stream.binary(b"hello");
    assert_eq!(stream.next().await, Heard::Binary(Bytes::from_static(b"echo:hello")));
    stream.binary(b"again");
    assert_eq!(stream.next().await, Heard::Binary(Bytes::from_static(b"echo:again")));
    // The stream's request names it by the id its activity gave it.
    let opened = [("live_view".to_owned(), "s1".to_owned())];
    assert_eq!(fake.held.lock().unwrap().streams, opened);

    let mut unknown = page.open(header("stream", json!({ "stream": "terminal" }))).await;
    assert_eq!(unknown.next().await.json()["error"]["code"], "unknown_stream");
    // The open stream is the conversation's activity until the page closes
    // it; the refused one never was.
    assert_eq!(*fake.activity.borrow(), [told("c1", true)]);
    stream.close();
    until_told(&fake, 2).await;
    assert_eq!(*fake.activity.borrow(), [told("c1", true), told("c1", false)]);
}

fn told(conversation: &str, open: bool) -> (String, bool) {
    (conversation.to_owned(), open)
}

/// Waits until the runner told the backend `count` openings and closings
/// of streams.
async fn until_told(fake: &Fake, count: usize) {
    let mut activity = fake.activity.subscribe();
    activity.wait_for(|told| told.len() >= count).await.unwrap();
}

#[tokio::test]
async fn a_watch_carries_the_watch_and_its_paths_and_says_heartbeat_when_quiet() {
    let fake = Fake::default();
    let (says, said) = mpsc::unbounded_channel();
    *fake.watch_says.lock().unwrap() = Some(said);
    let direct = runner(&fake);
    let page = connected(&direct, "p1").await;

    let mut watch = page.open(header("watch", json!({}))).await;
    assert_eq!(watch.next().await.json(), json!({ "ok": true }));
    let live = FileWatchMessage::State {
        state: FileWatchState::Live,
        reason: None,
    };
    says.send(live).unwrap();
    assert_eq!(watch.next().await.json(), json!({ "type": "state", "state": "live" }));
    watch.text(r#"{"type":"paths","paths":["/etc/hosts"]}"#);
    // The fake's watch answers a `paths` message with `changed` for them.
    assert_eq!(watch.next().await.json()["paths"][0], "/etc/hosts");
    says.send(FileWatchMessage::Changed {
        paths: vec!["/work/a.rs".into(), "/work/server.log".into()],
        entries: vec!["/work/a.rs".into()],
        ignored: vec!["/work/server.log".into()],
    })
    .unwrap();
    assert_eq!(
        watch.next().await.json(),
        json!({ "type": "changed", "paths": ["/work/a.rs", "/work/server.log"], "entries": ["/work/a.rs"], "ignored": ["/work/server.log"] })
    );

    // Thirty quiet seconds, on a paused clock: the peer's own checks go on
    // meanwhile.
    tokio::time::pause();
    assert_eq!(watch.next().await.json(), json!({ "type": "heartbeat" }));
}

#[tokio::test]
async fn a_peer_keeps_at_most_its_channels_and_the_runner_its_peers() {
    let fake = Fake::default();
    let direct = runner(&fake);
    let page = connected(&direct, "p1").await;
    let mut held = Vec::new();
    // The offer's first channel is one of the peer's.
    for _ in 1..MAX_CHANNELS {
        // A watch that says nothing stays open.
        let mut watch = page.open(header("watch", json!({}))).await;
        assert_eq!(watch.next().await.json(), json!({ "ok": true }));
        held.push(watch);
    }
    let mut over = page.open(header("watch", json!({}))).await;
    assert_eq!(over.next().await.json()["error"]["code"], "busy");
    assert_eq!(over.next().await, Heard::Closed);

    let mut pages = Vec::new();
    for peer in 1..MAX_PEERS {
        pages.push(Page::offer(&direct, &format!("q{peer}"), introduction()).await.unwrap());
    }
    let refused = Page::offer(&direct, "one-too-many", introduction()).await.err().unwrap();
    assert_eq!(refused.code, OfferRefusal::Busy);
    // A new offer for a peer it keeps replaces that one.
    assert!(Page::offer(&direct, "q1", introduction()).await.is_ok());
}

#[tokio::test]
async fn closing_a_peer_ends_its_connection_and_closing_all_ends_every_one() {
    let fake = Fake::default();
    let direct = runner(&fake);
    let mut first = connected(&direct, "p1").await;
    let mut second = connected(&direct, "p2").await;
    let mut stream = first.open(header("stream", json!({ "stream": "browser" }))).await;
    assert_eq!(stream.next().await.json(), json!({ "ok": true }));
    let mut other = second.open(header("stream", json!({ "stream": "browser" }))).await;
    assert_eq!(other.next().await.json(), json!({ "ok": true }));

    // The streams end with their peers, and the backend hears each end.
    direct.close("p1");
    assert_eq!(stream.next().await, Heard::Closed, "its channels end");
    first.wait_closed().await;
    let mut peers = direct.peers();
    peers.wait_for(|peers| *peers == 1).await.unwrap();
    until_told(&fake, 3).await;
    assert_eq!(fake.activity.borrow()[2], told("c1", false));

    direct.close_all();
    second.wait_closed().await;
    peers.wait_for(|peers| *peers == 0).await.unwrap();
    until_told(&fake, 4).await;
    assert_eq!(fake.activity.borrow()[3], told("c1", false));
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_does_not_connect_within_ten_seconds_is_given_up() {
    let fake = Fake::default();
    let direct = runner(&fake);
    let silent = Page::offer(&direct, "p1", introduction()).await.unwrap();
    // The page never sends a check: it is gone.
    drop(silent);
    let mut peers = direct.peers();
    peers.wait_for(|peers| *peers == 1).await.unwrap();
    let started = tokio::time::Instant::now();
    peers.wait_for(|peers| *peers == 0).await.unwrap();
    assert_eq!(started.elapsed().as_secs(), 10);
}

// Tens of milliseconds over loopback.
#[tokio::test]
async fn the_probe_channel_echoes_each_probe() {
    let fake = Fake::default();
    let direct = runner(&fake);
    let page = connected(&direct, "p1").await;

    let mut probe = page.probe().await;
    for id in [1, 2, 3] {
        let sent = json!({ "id": id }).to_string();
        probe.text(&sent);
        assert_eq!(probe.next().await, Heard::Text(sent), "each probe comes back as it went");
    }
}

/// A STUN server on 127.0.0.1 that answers every binding request with
/// `mapped` as its XOR-MAPPED-ADDRESS, the address it says it saw the
/// request come from, as a public server answers a request that crossed a
/// router (RFC 8489 § 14.2).
async fn stun_server(mapped: SocketAddrV4) -> (SocketAddr, AbortOnDropHandle<()>) {
    const COOKIE: u32 = 0x2112_A442;
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let serving = tokio::spawn(async move {
        let mut buffer = vec![0; 2048];
        loop {
            let (length, from) = socket.recv_from(&mut buffer).await.unwrap();
            let request = &buffer[..length];
            if length < 20 || request[..2] != [0x00, 0x01] {
                continue;
            }
            let mut reply = vec![0x01, 0x01, 0x00, 12];
            reply.extend_from_slice(&request[4..20]);
            reply.extend_from_slice(&[0x00, 0x20, 0x00, 8, 0x00, 0x01]);
            reply.extend_from_slice(&(mapped.port() ^ (COOKIE >> 16) as u16).to_be_bytes());
            reply.extend_from_slice(&(u32::from(*mapped.ip()) ^ COOKIE).to_be_bytes());
            socket.send_to(&reply, from).await.unwrap();
        }
    });
    (address, AbortOnDropHandle::new(serving))
}

// Tens of milliseconds: a STUN answer and a connection over loopback.
#[tokio::test]
async fn a_peer_offers_the_page_the_public_address_a_stun_server_saw_it_at() {
    let fake = Fake::default();
    let direct = runner(&fake);
    let public: SocketAddrV4 = "203.0.113.9:4444".parse().unwrap();
    let (server, _serving) = stun_server(public).await;
    let stun: StunUrl = format!("stun:{server}").parse().unwrap();
    let mut found = None;
    let mut page = Page::connect(async |offer| {
        let answered = direct.offer("p1".into(), offer, introduction(), vec![stun]).await?;
        found = Some(answered.candidates);
        Ok::<_, demi_runner_direct::Refused>(answered.sdp)
    })
    .await
    .unwrap();
    let candidate = tokio::time::timeout(std::time::Duration::from_secs(10), found.unwrap().recv())
        .await
        .expect("the peer offers its public address within the connect limit")
        .expect("the peer stands");
    assert!(
        candidate.starts_with("candidate:") && candidate.contains(" 203.0.113.9 4444 typ srflx "),
        "{candidate}"
    );
    // The candidate it found keeps nothing from connecting.
    assert!(page.wait_connected().await, "the peer connects");
}

// Tens of milliseconds over loopback; without the trickled candidate the
// peer gives up after ten seconds.
#[tokio::test]
async fn a_candidate_the_page_trickles_after_the_answer_makes_the_connection() {
    let fake = Fake::default();
    let direct = runner(&fake);
    // Neither end's description names an address: the page's candidate
    // reaches the runner only as it trickles it.
    let mut page = Page::trickled(&direct, "p1", introduction()).await.unwrap();
    assert!(page.wait_connected().await, "the runner's checks to the trickled candidate connect");
    let mut list = page.open(header("list", json!({}))).await;
    assert_eq!(list.next().await.json()["path"], "/work", "its channels carry operations");
}

fn interface(name: &str, ip: [u8; 4], up: bool, broadcast: bool, p2p: bool) -> Interface {
    Interface {
        name: name.into(),
        addr: IfAddr::V4(Ifv4Addr {
            ip: Ipv4Addr::from(ip),
            netmask: Ipv4Addr::new(255, 255, 255, 0),
            prefixlen: 24,
            broadcast: broadcast.then(|| Ipv4Addr::new(ip[0], ip[1], ip[2], 255)),
        }),
        index: None,
        oper_status: if up { IfOperStatus::Up } else { IfOperStatus::Down },
        is_p2p: p2p,
        #[cfg(windows)]
        adapter_name: String::new(),
    }
}

#[test]
fn the_runner_offers_loopback_and_its_local_networks_never_a_tunnel() {
    let interfaces = [
        interface("lo0", [127, 0, 0, 1], true, false, false),
        interface("en0", [192, 168, 1, 20], true, true, false),
        interface("utun4", [198, 18, 0, 1], true, false, true),
        interface("tun0", [10, 8, 0, 2], true, true, true),
        interface("en1", [10, 0, 0, 5], false, true, false),
        interface("bridge0", [192, 168, 64, 1], true, true, false),
    ];
    assert_eq!(
        offered(&interfaces),
        [
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::new(192, 168, 1, 20),
            Ipv4Addr::new(192, 168, 64, 1),
        ]
    );
}

