//! The browser suite through the whole stack (`scenarios.md` § Browser
//! suite): the backend, a paired device's real runner, the `demi.browser`
//! package the workspace built, and the pinned Chrome for Testing that
//! `DEMI_TEST_CHROME` names, with a scripted model. An ordinary run ignores
//! it; it runs as an ordinary user, since Chrome refuses root on Linux with
//! its sandbox.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::{Buf as _, BytesMut};
use demi_command_package_browser_protocol::live::{CONTROL_FRAME, VIDEO_FRAME, VideoHeader};
use demi_provider_common::testing::MockVendor;
use futures_util::{SinkExt as _, StreamExt as _};
use reqwest::StatusCode;
use serde_json::{Value, json};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::task::AbortOnDropHandle;

use crate::conversations::{anthropic_at, create};
use crate::streams::{self, Socket};
use crate::support::{Harness, chrome_resource, eventually};
use crate::work::{Driven, say, shell, switch};

const CONVERSATION: &str = "7b6a5c4d-8f3a-4c1e-9d2b-7a1c2e3f4a01";

/// What the fixture page says, which the agent reads back.
const GREETING: &str = "Hello from the fixture page";

/// Serves one page over HTTP on the loopback interface until dropped.
struct Page {
    url: String,
    _serving: AbortOnDropHandle<()>,
}

impl Page {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let body = format!(
            "<!doctype html><title>Fixture</title><h1>{GREETING}</h1>\
             <div style=\"width: 40px; height: 40px; background: red\"></div>"
        );
        let serving = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    // Reads the request's head; the page answers every
                    // request the same.
                    let mut head = Vec::new();
                    let mut buffer = [0; 1024];
                    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                        match socket.read(&mut buffer).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => head.extend_from_slice(&buffer[..read]),
                        }
                    }
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\n\
                         connection: close\r\n\r\n{body}",
                        body.len()
                    );
                    // A browser that went away needs no answer.
                    let _gone = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        Self {
            url,
            _serving: AbortOnDropHandle::new(serving),
        }
    }
}

/// The processes running a Chrome executable installed under `root`.
fn chrome_processes(root: &Path) -> Vec<sysinfo::Pid> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    system
        .processes()
        .iter()
        .filter(|(_, process)| process.exe().is_some_and(|exe| exe.starts_with(root)))
        .map(|(pid, _)| *pid)
        .collect()
}

/// One frame of the live view, as `live-view.md` § Framing and versions
/// gives it.
enum Frame {
    Control(Value),
    Video(VideoHeader, Vec<u8>),
}

/// A page's view of the conversation's browser over the `browser` user
/// stream: it frames its messages and splits the module's out of the
/// socket's bytes, which carry no message boundaries.
struct View {
    socket: Socket,
    pending: BytesMut,
}

impl View {
    async fn send(&mut self, message: Value) {
        let json = serde_json::to_vec(&message).unwrap();
        let mut frame = Vec::with_capacity(5 + json.len());
        frame.extend_from_slice(&u32::try_from(1 + json.len()).unwrap().to_be_bytes());
        frame.push(CONTROL_FRAME);
        frame.extend_from_slice(&json);
        self.socket
            .send(Message::Binary(frame.into()))
            .await
            .unwrap();
    }

    async fn next(&mut self) -> Frame {
        loop {
            if self.pending.len() >= 4 {
                let length = u32::from_be_bytes(self.pending[..4].try_into().unwrap()) as usize;
                if self.pending.len() >= 4 + length {
                    self.pending.advance(4);
                    let mut frame = self.pending.split_to(length);
                    let kind = frame.get_u8();
                    return match kind {
                        CONTROL_FRAME => Frame::Control(serde_json::from_slice(&frame).unwrap()),
                        VIDEO_FRAME => {
                            let (header, data) = VideoHeader::split(&frame).unwrap();
                            Frame::Video(header, data.to_vec())
                        }
                        kind => panic!("a frame of unknown kind {kind}"),
                    };
                }
            }
            let message = tokio::time::timeout(Duration::from_secs(30), self.socket.next())
                .await
                .expect("the live view went quiet")
                .expect("the live view ended")
                .unwrap();
            match message {
                Message::Binary(bytes) => self.pending.extend_from_slice(&bytes),
                Message::Close(close) => panic!("the live view closed: {close:?}"),
                _ => {}
            }
        }
    }

    /// The next control message whose `type` is `kind`.
    async fn control(&mut self, kind: &str) -> Value {
        loop {
            if let Frame::Control(message) = self.next().await
                && message["type"] == kind
            {
                return message;
            }
        }
    }

    /// The first picture of the stream `generation` names, or of the stream
    /// that replaces it, as when the tab takes the panel's size; each
    /// picture is acknowledged as the page acknowledges what it shows.
    async fn picture(&mut self, mut generation: u64) -> (VideoHeader, Vec<u8>) {
        loop {
            match self.next().await {
                Frame::Control(message) if message["type"] == "stream" => {
                    generation = message["generation"].as_u64().unwrap();
                }
                Frame::Control(_) => {}
                Frame::Video(header, data) => {
                    self.send(json!({
                        "type": "ack", "generation": header.generation,
                        "sequence": header.sequence, "decodeQueue": 0,
                    }))
                    .await;
                    if u64::from(header.generation) == generation {
                        return (header, data);
                    }
                }
            }
        }
    }
}

/// An agent on a paired device opens a page served on the loopback
/// interface with `demi browser` and reads it, the user watches the tab
/// through the backend's `browser` user stream, and archiving the
/// conversation releases it on the device, which leaves no Chrome process
/// (`browser.md` § Release, `live-view.md` § The stream). About 13 s here: the
/// device installs the package, Chrome starts, and the view's first picture
/// is encoded.
#[tokio::test]
#[ignore = "the browser suite: needs DEMI_TEST_CHROME and an ordinary user (scenarios.md § Browser suite)"]
async fn an_agent_drives_chrome_on_a_paired_device_which_the_user_watches_until_release() {
    let chrome = PathBuf::from(std::env::var_os("DEMI_TEST_CHROME").expect("DEMI_TEST_CHROME"));
    let page = Page::start().await;
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_browser_package();
    let (backend, master) = harness.start_set_up().await;
    let laptop = backend.pair(&master, "laptop").await;
    // The device's runner finds the package's Chrome installed in its
    // artifact cache, where it would unpack the archive.
    let resource = chrome_resource();
    let artifacts = laptop.runner.state_dir().join("artifacts");
    // Where the device's Chrome runs from.
    let browsers = artifacts.join(&resource.sha256);
    let archive = demi_shared_artifacts::Archive {
        digest: demi_shared_artifacts::Digest {
            size: resource.size,
            sha256: resource.sha256,
        },
        entry: resource.entry,
    };
    demi_shared_artifacts::testing::install_unpacked(
        &artifacts,
        &archive,
        &chrome,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("DEMI_TEST_CHROME names an unpacked copy of the pinned release");
    let provider = anthropic_at(&backend, &master, &vendor, "/laptop").await;
    create(&backend, &master, CONVERSATION).await;
    let home = laptop.runner.home_dir().to_owned();
    switch(&backend, &master, CONVERSATION, &laptop, &home).await;
    let mut work = Driven::open(
        &backend,
        &master,
        &vendor,
        CONVERSATION,
        &provider,
        "/laptop",
    )
    .await;

    let script = format!(
        "demi browser open {} && demi browser content read t1 --format text",
        page.url
    );
    let read = work
        .turn(vec![shell("browse", &script, 120_000), say("read")])
        .await;
    let output = &read.received[0];
    assert!(output.contains("exitCode: 0"), "{output}");
    assert!(output.contains("Tab: t1"), "{output}");
    assert!(output.contains(GREETING), "{output}");
    assert!(
        !chrome_processes(&browsers).is_empty(),
        "the device runs the conversation's Chrome"
    );

    // The user watches the agent's tab: its pictures reach the page through
    // the backend.
    let mut view = View {
        socket: streams::socket(&backend, &master, CONVERSATION, "browser").await,
        pending: BytesMut::new(),
    };
    view.send(json!({"type": "hello", "platform": "linux"}))
        .await;
    view.send(json!({
        "type": "panel", "width": 800, "height": 600, "devicePixelRatio": 1,
        "screenWidth": 1440, "screenHeight": 900,
    }))
    .await;
    let state = view.control("state").await;
    assert_eq!(state["tabs"][0]["id"], "t1", "{state}");
    view.send(json!({"type": "watch", "tab": "t1"})).await;
    let stream = view.control("stream").await;
    assert_eq!(stream["tab"], "t1", "{stream}");
    let (header, data) = view.picture(stream["generation"].as_u64().unwrap()).await;
    assert_eq!(header.tab.as_str(), "t1");
    assert!(header.key, "a stream starts with a key frame");
    assert_eq!(&data[..4], [0, 0, 0, 1], "H.264 Annex B");
    drop(view);

    // Archiving the conversation releases it on the device, which retires
    // its browser.
    let archived = backend
        .patch(
            &format!("/api/conversations/{CONVERSATION}"),
            &master,
            json!({ "archived": true }),
        )
        .await;
    assert_eq!(
        archived.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&archived.body)
    );
    eventually("the released conversation's Chrome is gone", || {
        let running = chrome_processes(&browsers);
        async move { running.is_empty() }
    })
    .await;
    drop(laptop);
    backend.close().await;
}
