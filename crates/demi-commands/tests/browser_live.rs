//! The live view against real Chrome (`browser-live-view.md` § Acceptance):
//! a viewer drives `browser.live` as the page does, beside the agent's
//! commands on the same tabs.

mod browser_families;

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    http::{HeaderMap, header::USER_AGENT},
    response::Html,
    routing::get,
};
use browser_families::{BrowserFixture, with_browser_fixture};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use demi_command_service::{
    Input, ServiceError,
    protocol::{CommandCaller, Completion, Record},
};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

const PAGE: &str = r#"<!doctype html>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
body { margin: 0; font: 16px sans-serif; height: 3000px }
#spin { position: absolute; left: 0; top: 400px; width: 40px; height: 40px; background: red }
</style>
<input id="text" style="position: absolute; left: 10px; top: 10px; width: 300px; height: 30px">
<textarea id="area" style="position: absolute; left: 10px; top: 60px; width: 300px; height: 60px"></textarea>
<select id="choice" style="position: absolute; left: 10px; top: 140px; width: 200px; height: 30px">
  <option value="a">A</option><option value="b">B</option>
</select>
<input id="file" type="file" style="position: absolute; left: 10px; top: 190px">
<button id="alert" style="position: absolute; left: 10px; top: 240px; width: 100px; height: 30px"
  onclick="alert('hello')">Alert</button>
<p id="copy" style="position: absolute; left: 10px; top: 290px">copy me</p>
<div id="stripes" style="position: absolute; left: 150px; top: 380px; width: 200px; height: 60px;
  background: repeating-linear-gradient(90deg, #000 0, #000 .5px, #fff .5px, #fff 1px)"></div>
<button id="write" style="position: absolute; left: 10px; top: 340px; width: 100px; height: 30px"
  onclick="navigator.clipboard.writeText('written').then(() => window.wrote = 'ok', error => window.wrote = String(error))">Write</button>
<div id="spin"></div>
<script>
window.events = [];
for (const type of ['keydown', 'keypress', 'keyup', 'input', 'change', 'paste', 'touchstart', 'touchend', 'mousedown', 'mouseup'])
  addEventListener(type, event => events.push(`${type}:${event.key ?? event.target.id ?? ''}`), true);
addEventListener('paste', event => {
  window.pasted = { text: event.clipboardData.getData('text/plain'), html: event.clipboardData.getData('text/html') };
});
let x = 0;
setInterval(() => { spin.style.left = `${(x = (x + 5) % 500)}px`; }, 16);
</script>"#;

struct Site {
    base: String,
    /// The user agent of every request for the page, in order.
    served: Arc<Mutex<Vec<String>>>,
    _task: AbortOnDropHandle<()>,
}

async fn site() -> Site {
    let served = Arc::new(Mutex::new(Vec::new()));
    let recording = served.clone();
    let app = Router::new()
        .route(
            "/",
            get(move |headers: HeaderMap| {
                let agent = headers
                    .get(USER_AGENT)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                recording.lock().unwrap().push(agent);
                async { Html(PAGE) }
            }),
        )
        // A page that does not answer while a test runs.
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(120)).await;
                Html(PAGE)
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let task = AbortOnDropHandle::new(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    Site {
        base,
        served,
        _task: task,
    }
}

#[derive(Debug)]
enum Frame {
    Control(Value),
    Video {
        tab: String,
        generation: u32,
        sequence: u32,
        key: bool,
        width: u16,
        height: u16,
        data: Bytes,
    },
}

/// A viewer as the page is one: it frames its messages and splits the
/// module's.
struct View {
    input: Option<mpsc::UnboundedSender<Result<Bytes, ServiceError>>>,
    frames: mpsc::UnboundedReceiver<Frame>,
    completion: tokio::task::JoinHandle<Result<Completion, ServiceError>>,
    _reading: AbortOnDropHandle<()>,
}

impl View {
    fn open(fixture: &BrowserFixture) -> Self {
        let (input, receiver) = mpsc::unbounded_channel();
        let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|item| (item, receiver))
        });
        let (mut records, invoke) = fixture.start(
            "browser.live",
            json!({}),
            user(),
            Input::from_stream(stream),
            CancellationToken::new(),
        );
        let (frames, received) = mpsc::unbounded_channel();
        let reading = tokio::spawn(async move {
            let mut pending = BytesMut::new();
            while let Some(record) = records.recv().await {
                let Record::Stdout(bytes) = record else {
                    panic!("a view writes only its stream");
                };
                pending.extend_from_slice(&bytes);
                while pending.len() >= 4 {
                    let length = u32::from_be_bytes(pending[..4].try_into().unwrap()) as usize;
                    if pending.len() < 4 + length {
                        break;
                    }
                    pending.advance(4);
                    let mut frame = pending.split_to(length).freeze();
                    let parsed = match frame.get_u8() {
                        1 => Frame::Control(serde_json::from_slice(&frame).unwrap()),
                        2 => {
                            let tab = String::from_utf8(frame.split_to(24).to_vec()).unwrap();
                            let generation = frame.get_u32();
                            let sequence = frame.get_u32();
                            let key = frame.get_u8() == 1;
                            frame.advance(3 + 8);
                            let width = frame.get_u16();
                            let height = frame.get_u16();
                            Frame::Video {
                                tab,
                                generation,
                                sequence,
                                key,
                                width,
                                height,
                                data: frame,
                            }
                        }
                        kind => panic!("unknown frame kind {kind}"),
                    };
                    if frames.send(parsed).is_err() {
                        return;
                    }
                }
            }
        });
        Self {
            input: Some(input),
            frames: received,
            completion: tokio::spawn(invoke),
            _reading: AbortOnDropHandle::new(reading),
        }
    }

    fn send(&self, message: Value) {
        let json = serde_json::to_vec(&message).unwrap();
        let mut frame = BytesMut::new();
        frame.put_u32(1 + json.len() as u32);
        frame.put_u8(1);
        frame.put_slice(&json);
        self.raw(frame.freeze());
    }

    fn raw(&self, bytes: Bytes) {
        self.input.as_ref().unwrap().send(Ok(bytes)).unwrap();
    }

    fn file(&self, upload: u32, file: u32, data: &[u8]) {
        let mut frame = BytesMut::new();
        frame.put_u32(1 + 8 + data.len() as u32);
        frame.put_u8(3);
        frame.put_u32(upload);
        frame.put_u32(file);
        frame.put_slice(data);
        self.raw(frame.freeze());
    }

    /// The next frame, acknowledging video as the page shows it.
    async fn next(&mut self) -> Frame {
        let frame = tokio::time::timeout(Duration::from_secs(30), self.frames.recv())
            .await
            .expect("the module went quiet")
            .expect("the view ended");
        if let Frame::Video {
            generation,
            sequence,
            ..
        } = &frame
        {
            self.send(json!({"type": "ack", "generation": generation, "sequence": sequence, "decodeQueue": 0}));
        }
        frame
    }

    /// The next control message that `wanted`, described by `what`, accepts.
    async fn until(&mut self, what: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            if let Frame::Control(message) = self.next().await {
                if wanted(&message) {
                    return message;
                }
                if message["type"] != "heartbeat" {
                    seen.push(message);
                }
            }
        }
        panic!("the module never sent {what}; it sent: {seen:?}");
    }

    async fn message(&mut self, kind: &str) -> Value {
        self.until(kind, |message| message["type"] == kind).await
    }

    /// The first video frame after the stream message of `generation`.
    async fn picture(&mut self, generation: u64) -> (String, bool, u16, u16, Bytes) {
        loop {
            if let Frame::Video {
                tab,
                generation: arrived,
                key,
                width,
                height,
                data,
                ..
            } = self.next().await
                && u64::from(arrived) == generation
            {
                return (tab, key, width, height, data);
            }
        }
    }

    fn pointer(&self, tab: &str, action: &str, x: f64, y: f64) {
        let pressed = action == "down";
        self.send(json!({
            "type": "pointer", "tab": tab, "action": action, "x": x, "y": y,
            "button": if action == "move" { "none" } else { "left" },
            "buttons": u8::from(pressed), "clickCount": u8::from(action != "move"), "modifiers": 0,
        }));
    }

    fn click(&self, tab: &str, x: f64, y: f64) {
        self.pointer(tab, "down", x, y);
        self.pointer(tab, "up", x, y);
    }

    fn key(
        &self,
        tab: &str,
        key: &str,
        code: &str,
        key_code: u8,
        modifiers: u8,
        text: Option<&str>,
    ) {
        for action in ["down", "up"] {
            let mut message = json!({
                "type": "key", "tab": tab, "action": action, "key": key, "code": code,
                "keyCode": key_code, "modifiers": modifiers, "repeat": false, "location": 0,
                "altGraph": false,
            });
            if action == "down"
                && let Some(text) = text
            {
                message["text"] = json!(text);
            }
            self.send(message);
        }
    }

    /// The page ends its side; the view completes.
    async fn close(mut self) -> Completion {
        self.input.take();
        tokio::time::timeout(Duration::from_secs(30), self.completion)
            .await
            .expect("the view did not end")
            .unwrap()
            .unwrap()
    }
}

async fn evaluate(fixture: &BrowserFixture, tab: &str, expression: &str) -> Value {
    fixture
        .call(
            "browser.eval",
            json!({"tab": tab, "expression": expression}),
        )
        .await["value"]
        .clone()
}

/// Polls the page until `expression` is true.
async fn eventually(fixture: &BrowserFixture, tab: &str, expression: &str) {
    for _ in 0..200 {
        // A document that is being replaced refuses the evaluation.
        let (_, result) = fixture
            .result(
                "browser.eval",
                json!({"tab": tab, "expression": expression}),
                CancellationToken::new(),
            )
            .await;
        if result["value"] == json!(true) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let events = evaluate(fixture, tab, "window.events").await;
    panic!("never true: {expression}; events: {events}");
}

/// The caller of the page's view and requests.
fn user() -> CommandCaller {
    serde_json::from_value(json!({"kind": "user"})).unwrap()
}

/// The exit code and answer of `operation` run as the page's request runs
/// it: for a `user` caller (`web-api.md` § Conversation browser tabs).
async fn user_result(fixture: &BrowserFixture, operation: &str, args: Value) -> (u8, Value) {
    fixture
        .result_for(
            user(),
            operation,
            args,
            CancellationToken::new(),
            Vec::new(),
        )
        .await
}

/// The answer of a request that succeeds.
async fn request(fixture: &BrowserFixture, operation: &str, args: Value) -> Value {
    let (code, result) = user_result(fixture, operation, args).await;
    assert_eq!(code, 0, "{operation}: {result}");
    result
}

/// A viewer at ratio 2 in an 800 × 600 panel.
fn hello(view: &View, platform: &str) {
    view.send(json!({"type": "hello", "platform": platform}));
    view.send(json!({
        "type": "panel", "width": 800, "height": 600, "devicePixelRatio": 2,
        "screenWidth": 1440, "screenHeight": 900,
    }));
}

/// Watches `tab` until its pictures arrive at the panel's size.
async fn watch(view: &mut View, tab: &str) -> u64 {
    view.send(json!({"type": "watch", "tab": tab}));
    let stream = view
        .until("a 1600-wide stream", |message| {
            message["type"] == "stream" && message["width"] == 1600
        })
        .await;
    assert_eq!(
        (stream["tab"].as_str(), stream["height"].as_u64()),
        (Some(tab), Some(1200))
    );
    let generation = stream["generation"].as_u64().unwrap();
    let (pictured, key, width, height, data) = view.picture(generation).await;
    assert!(key, "a stream starts with a key frame");
    assert_eq!(pictured, tab, "a frame names its tab");
    assert_eq!((width, height), (1600, 1200));
    assert_eq!(&data[..4], [0, 0, 0, 1], "Annex B");
    generation
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_viewer_watches_a_tab_and_types_into_it_beside_the_agent() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let tab = fixture
            .call("browser.open", json!({"url": site.base, "timeout": 120000}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut view = View::open(&fixture);
        hello(&view, "linux");
        let state = view.message("state").await;
        assert_eq!(state["running"], true);
        assert_eq!(state["tabs"][0]["id"], tab);
        assert_eq!(state["tabs"][0]["createdBy"]["kind"], "agent");
        assert_eq!(state["watched"], Value::Null);
        watch(&mut view, &tab).await;
        // The agent sees the viewer's panel as the tab's viewport.
        let info = fixture.call("browser.info", json!({"tab": tab})).await;
        assert_eq!(
            info["viewport"],
            json!({"width": 800, "height": 600, "devicePixelRatio": 2.0, "mode": "web"})
        );
        // The page renders at the viewer's ratio on the viewer's screen.
        let screen = evaluate(&fixture, &tab, "[devicePixelRatio, screen.width, screen.height, innerWidth, outerWidth >= innerWidth, outerHeight > innerHeight]").await;
        assert_eq!(screen, json!([2, 1440, 900, 800, true, true]));
        // A wheel turn scrolls the CSS distance the viewer turned.
        view.send(json!({"type": "wheel", "tab": tab, "x": 400, "y": 300, "deltaX": 0, "deltaY": 120, "modifiers": 0}));
        eventually(&fixture, &tab, "scrollY === 120").await;
        view.send(json!({"type": "wheel", "tab": tab, "x": 400, "y": 300, "deltaX": 0, "deltaY": -120, "modifiers": 0}));
        eventually(&fixture, &tab, "scrollY === 0").await;
        view.click(&tab, 100.0, 25.0);
        view.key(&tab, "h", "KeyH", 72, 0, Some("h"));
        view.key(&tab, "i", "KeyI", 73, 0, Some("i"));
        eventually(&fixture, &tab, "document.querySelector('#text').value === 'hi'").await;
        eventually(&fixture, &tab, "events.includes('keypress:h') && events.includes('input:text')").await;
        // Enter in a textarea is a newline.
        view.click(&tab, 100.0, 90.0);
        view.key(&tab, "a", "KeyA", 65, 0, Some("a"));
        view.key(&tab, "Enter", "Enter", 13, 0, None);
        view.key(&tab, "b", "KeyB", 66, 0, Some("b"));
        eventually(&fixture, &tab, "document.querySelector('#area').value === 'a\\nb'").await;
        // The agent's command and the user's input both take effect.
        fixture
            .call("browser.fill", json!({"tab": tab, "css": "#text", "text": "agent"}))
            .await;
        view.click(&tab, 100.0, 90.0);
        view.key(&tab, "c", "KeyC", 67, 0, Some("c"));
        eventually(&fixture, &tab, "document.querySelector('#text').value === 'agent' && document.querySelector('#area').value === 'a\\nbc'").await;
        assert_eq!(view.close().await.exit_code, 0);
        fixture
    })
    .await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn modes_follow_the_viewer_and_the_agent() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let tab = fixture
            .call("browser.open", json!({"url": site.base, "timeout": 120000}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut view = View::open(&fixture);
        hello(&view, "mac");
        view.message("state").await;
        watch(&mut view, &tab).await;
        view.send(json!({"type": "mode", "tab": tab, "mode": "mobile"}));
        let stream = view
            .until("a phone-wide stream", |message| message["type"] == "stream" && message["width"] == 780)
            .await;
        assert_eq!(stream["height"], 1688);
        // The page loads again, so the site serves it to a phone.
        for _ in 0..200 {
            if site.served.lock().unwrap().iter().any(|agent| agent.contains("Android")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let served = site.served.lock().unwrap().clone();
        assert!(
            served.last().is_some_and(|agent| agent.contains("Android")),
            "the page was never served to a phone: {served:?}"
        );
        eventually(&fixture, &tab, "document.readyState === 'complete'").await;
        let phone = evaluate(
            &fixture,
            &tab,
            "[navigator.userAgentData.platform, navigator.userAgentData.mobile, navigator.userAgentData.brands.some(brand => brand.brand === 'Chromium'), innerWidth, navigator.maxTouchPoints, navigator.userAgent.includes('Android')]",
        )
        .await;
        assert_eq!(phone, json!(["Android", true, true, 390, 5, true]));
        // A phone's clicks are taps.
        view.click(&tab, 100.0, 25.0);
        eventually(&fixture, &tab, "events.includes('touchstart:text') && events.includes('touchend:text')").await;
        // The agent sets a size; the view shows it as Custom.
        fixture
            .call(
                "browser.viewport.set",
                json!({"tab": tab, "width": 1000, "height": 700, "scale": 1}),
            )
            .await;
        let state = view
            .until("a custom viewport", |message| {
                message["type"] == "state" && message["tabs"][0]["viewport"]["mode"] == "custom"
            })
            .await;
        assert_eq!(
            state["tabs"][0]["viewport"],
            json!({"width": 1000, "height": 700, "devicePixelRatio": 1.0, "mode": "custom"})
        );
        let desktop = evaluate(&fixture, &tab, "[navigator.userAgent.includes('Android'), navigator.maxTouchPoints, innerWidth]").await;
        assert_eq!(desktop, json!([false, 0, 1000]));
        // Web discards the agent's size and takes the panel's again.
        view.send(json!({"type": "mode", "tab": tab, "mode": "web"}));
        view.until("a 1600-wide stream", |message| message["type"] == "stream" && message["width"] == 1600)
            .await;
        let info = fixture.call("browser.info", json!({"tab": tab})).await;
        assert_eq!(info["viewport"]["mode"], "web");
        assert_eq!(view.close().await.exit_code, 0);
        fixture
    })
    .await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn dialogs_controls_files_and_the_clipboard_reach_the_viewer() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let tab = fixture
            .call("browser.open", json!({"url": site.base, "timeout": 120000}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut view = View::open(&fixture);
        hello(&view, "linux");
        view.message("state").await;
        watch(&mut view, &tab).await;
        // Native controls, with identities and revisions.
        let controls = view
            .until("the page's two controls", |message| message["type"] == "controls" && message["controls"].as_array().is_some_and(|controls| controls.len() == 2))
            .await;
        let select = controls["controls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|control| control["kind"] == "select")
            .unwrap()
            .clone();
        assert_eq!(select["options"][1]["label"], "B");
        view.send(json!({
            "type": "choice", "tab": tab, "token": select["token"], "revision": select["revision"],
            "value": "b", "indices": [1],
        }));
        let chosen = view.message("choice").await;
        assert_eq!(chosen, json!({"type": "choice", "token": select["token"], "accepted": true}));
        eventually(&fixture, &tab, "document.querySelector('#choice').value === 'b' && events.includes('change:choice')").await;
        // A stale revision is refused.
        view.send(json!({
            "type": "choice", "tab": tab, "token": select["token"], "revision": 99,
            "value": "a", "indices": [0],
        }));
        assert_eq!(view.message("choice").await["accepted"], false);
        // Files travel through the stream.
        let file = controls["controls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|control| control["kind"] == "file")
            .unwrap()
            .clone();
        view.send(json!({
            "type": "upload", "tab": tab, "token": file["token"], "revision": file["revision"], "upload": 7,
            "files": [{"name": "notes.txt", "mimeType": "text/plain", "size": 11}],
        }));
        view.file(7, 0, b"hello ");
        view.file(7, 0, b"world");
        assert_eq!(view.message("choice").await["accepted"], true);
        eventually(&fixture, &tab, "document.querySelector('#file').files[0]?.name === 'notes.txt' && document.querySelector('#file').files[0].size === 11").await;
        // A dialog shows in the view; the first answer wins.
        view.click(&tab, 50.0, 255.0);
        let dialog = view
            .until("an open dialog", |message| message["type"] == "dialog" && !message["dialog"].is_null())
            .await;
        assert_eq!(
            dialog["dialog"],
            json!({"type": "alert", "message": "hello", "defaultText": ""})
        );
        view.send(json!({"type": "dialog", "tab": tab, "accept": true}));
        view.until("the dialog answered", |message| message["type"] == "dialog" && message["dialog"].is_null())
            .await;
        // A paste is a real paste event.
        view.click(&tab, 100.0, 90.0);
        view.send(json!({"type": "paste", "tab": tab, "text": "pasted", "html": "<b>pasted</b>"}));
        eventually(&fixture, &tab, "window.pasted?.text === 'pasted' && document.querySelector('#area').value === 'pasted'").await;
        // Text the page copies after the viewer's input reaches the viewer.
        // The click leaves the textarea, as a user copying elsewhere does.
        view.click(&tab, 50.0, 300.0);
        fixture
            .call(
                "browser.select-text",
                json!({"tab": tab, "css": "#copy", "text": "copy me"}),
            )
            .await;
        // The Host's copy shortcut.
        let command = if cfg!(target_os = "macos") { 4 } else { 2 };
        view.key(&tab, "c", "KeyC", 67, command, None);
        let copied = view.message("clipboard").await;
        assert_eq!(copied["text"], "copy me");
        // So does what the page writes with the Clipboard API, once.
        // Selecting the text scrolled the page; the button is where the
        // viewer sees it again.
        view.send(json!({"type": "wheel", "tab": tab, "x": 400, "y": 300, "deltaX": 0, "deltaY": -200, "modifiers": 0}));
        eventually(&fixture, &tab, "scrollY === 0").await;
        view.click(&tab, 50.0, 355.0);
        let written = view.message("clipboard").await;
        assert_eq!(written["text"], "written");
        assert_eq!(view.close().await.exit_code, 0);
        fixture
    })
    .await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_view_waits_for_a_browser_and_ends_with_its_last_tab() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let mut view = View::open(&fixture);
        hello(&view, "linux");
        let state = view.message("state").await;
        assert_eq!(
            state,
            json!({"type": "state", "running": false, "tabs": [], "watched": null})
        );
        // The page opens a tab with a request; the waiting view hears of the
        // browser that request started, and watches nothing until it asks.
        let opened = request(&fixture, "browser.open", json!({"url": site.base})).await;
        let tab = opened["tab"].as_str().unwrap().to_owned();
        let state = view
            .until("the state listing the tab", |message| {
                message["type"] == "state" && message["tabs"][0]["id"] == json!(tab)
            })
            .await;
        assert_eq!(state["running"], true);
        assert_eq!(state["tabs"][0]["createdBy"], json!({"kind": "user"}));
        assert_eq!(state["watched"], Value::Null);
        watch(&mut view, &tab).await;
        // Another tab while the first is watched: the view hears of it and
        // keeps the tab it watches until the page asks for the other.
        let second = request(&fixture, "browser.open", json!({"url": site.base})).await;
        let second = second["tab"].as_str().unwrap().to_owned();
        let state = view
            .until("the state listing both tabs", |message| {
                message["type"] == "state"
                    && message["tabs"]
                        .as_array()
                        .is_some_and(|tabs| tabs.len() == 2)
            })
            .await;
        assert_eq!(state["watched"].as_str(), Some(tab.as_str()));
        view.send(json!({"type": "watch", "tab": second}));
        let stream = view
            .until("a stream of the second tab", |message| {
                message["type"] == "stream"
                    && message["tab"] == json!(second)
                    && message["width"] == 1600
            })
            .await;
        let (pictured, ..) = view.picture(stream["generation"].as_u64().unwrap()).await;
        assert_eq!(pictured, second, "the second tab is the one it sees");
        let closed = request(&fixture, "browser.close", json!({"tab": second})).await;
        assert_eq!(closed, json!({"closed": second}));
        let closed = request(&fixture, "browser.close", json!({"tab": tab})).await;
        assert_eq!(closed, json!({"closed": tab}));
        let ended = view.message("ended").await;
        assert_eq!(ended["reason"], "browser_ended");
        assert_eq!(view.close().await.exit_code, 0);
        let tabs = fixture.call("browser.tabs", json!({})).await;
        assert_eq!(tabs["tabs"], json!([]));
        fixture
    })
    .await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn the_users_requests_answer_without_waiting_for_a_page() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let slow = format!("{}slow", site.base);
        let second = format!("{}?second", site.base);
        // Far less than the slow page takes, and enough for a busy machine.
        let prompt = Duration::from_secs(10);

        let blank = request(&fixture, "browser.open", json!({"url": "about:blank"})).await;
        let tab = blank["tab"].as_str().unwrap().to_owned();
        assert_eq!(blank, json!({"tab": tab, "url": "about:blank"}));
        let started = Instant::now();
        let loading = request(&fixture, "browser.open", json!({"url": slow})).await;
        assert!(started.elapsed() < prompt, "open waited for its page");
        let other = loading["tab"].as_str().unwrap().to_owned();
        assert_eq!(loading, json!({"tab": other, "url": slow}));
        let tabs = request(&fixture, "browser.tabs", json!({})).await;
        let listed: Vec<_> = tabs["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (row["id"].as_str().unwrap(), row["createdBy"].clone()))
            .collect();
        let user = json!({"kind": "user"});
        assert_eq!(
            listed,
            [(tab.as_str(), user.clone()), (other.as_str(), user)]
        );

        // A blank tab has nowhere to go back to.
        let (code, refused) = user_result(&fixture, "browser.back", json!({"tab": tab})).await;
        assert_eq!(
            (code, &refused["error"]["code"]),
            (1, &json!("history_boundary"))
        );
        let (code, refused) = user_result(
            &fixture,
            "browser.goto",
            json!({"tab": tab, "url": "javascript:1"}),
        )
        .await;
        assert_eq!(
            (code, &refused["error"]["code"]),
            (2, &json!("invalid_input"))
        );

        let at = |url: &str| {
            format!(
                "location.href === {} && document.readyState === 'complete'",
                json!(url)
            )
        };
        let went = request(
            &fixture,
            "browser.goto",
            json!({"tab": tab, "url": site.base}),
        )
        .await;
        assert_eq!(went, json!({"tab": tab, "url": site.base}));
        eventually(&fixture, &tab, &at(&site.base)).await;
        let went = request(&fixture, "browser.goto", json!({"tab": tab, "url": second})).await;
        assert_eq!(went, json!({"tab": tab, "url": second}));
        eventually(&fixture, &tab, &at(&second)).await;
        let back = request(&fixture, "browser.back", json!({"tab": tab})).await;
        assert_eq!(back, json!({"tab": tab, "url": site.base}));
        eventually(&fixture, &tab, &at(&site.base)).await;
        let forward = request(&fixture, "browser.forward", json!({"tab": tab})).await;
        assert_eq!(forward, json!({"tab": tab, "url": second}));
        eventually(&fixture, &tab, &at(&second)).await;
        let (code, refused) = user_result(&fixture, "browser.forward", json!({"tab": tab})).await;
        assert_eq!(
            (code, &refused["error"]["code"]),
            (1, &json!("history_boundary"))
        );
        let origin = evaluate(&fixture, &tab, "performance.timeOrigin").await;
        let reloaded = request(&fixture, "browser.reload", json!({"tab": tab})).await;
        assert_eq!(reloaded, json!({"tab": tab, "url": second}));
        // A new document starts its own clock.
        let renewed =
            format!("performance.timeOrigin > {origin} && document.readyState === 'complete'");
        eventually(&fixture, &tab, &renewed).await;

        let started = Instant::now();
        let went = request(&fixture, "browser.goto", json!({"tab": tab, "url": slow})).await;
        assert!(started.elapsed() < prompt, "goto waited for its page");
        assert_eq!(went, json!({"tab": tab, "url": slow}));

        let missing = "t_0000000000000000000000";
        let (code, refused) = user_result(&fixture, "browser.close", json!({"tab": missing})).await;
        assert_eq!(
            (code, &refused["error"]["code"]),
            (1, &json!("tab_not_found"))
        );
        for id in [&other, &tab] {
            let closed = request(&fixture, "browser.close", json!({"tab": id})).await;
            assert_eq!(closed, json!({"closed": id}));
        }
        let tabs = request(&fixture, "browser.tabs", json!({})).await;
        assert_eq!(tabs["tabs"], json!([]));
        fixture
    })
    .await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_viewer_that_ends_releases_only_what_it_holds() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let tab = fixture
            .call("browser.open", json!({"url": site.base, "timeout": 120000}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut view = View::open(&fixture);
        hello(&view, "linux");
        view.message("state").await;
        watch(&mut view, &tab).await;
        view.click(&tab, 100.0, 25.0);
        view.send(json!({
            "type": "key", "tab": tab, "action": "down", "key": "Shift", "code": "ShiftLeft",
            "keyCode": 16, "modifiers": 8, "repeat": false, "location": 1, "altGraph": false,
        }));
        view.pointer(&tab, "down", 400.0, 500.0);
        eventually(&fixture, &tab, "events.includes('keydown:Shift') && events.filter(event => event.startsWith('mousedown')).length === 2").await;
        assert_eq!(view.close().await.exit_code, 0);
        eventually(&fixture, &tab, "events.includes('keyup:Shift') && events.filter(event => event.startsWith('mouseup')).length === 2").await;
        // The agent's input goes on as before.
        fixture
            .call("browser.fill", json!({"tab": tab, "css": "#text", "text": "after"}))
            .await;
        eventually(&fixture, &tab, "document.querySelector('#text').value === 'after'").await;
        fixture
    })
    .await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn two_viewers_share_a_tab_and_the_last_to_operate_decides() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let tab = fixture
            .call("browser.open", json!({"url": site.base, "timeout": 120000}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut first = View::open(&fixture);
        hello(&first, "linux");
        first.message("state").await;
        watch(&mut first, &tab).await;
        let mut second = View::open(&fixture);
        second.send(json!({"type": "hello", "platform": "windows"}));
        second.send(json!({
            "type": "panel", "width": 1000, "height": 700, "devicePixelRatio": 1,
            "screenWidth": 1920, "screenHeight": 1080,
        }));
        second.message("state").await;
        // A second viewer joins the first's picture without changing it.
        second.send(json!({"type": "watch", "tab": tab}));
        let stream = second.message("stream").await;
        assert_eq!(
            (stream["width"].as_u64(), stream["height"].as_u64()),
            (Some(1600), Some(1200))
        );
        let (.., key, _, _, _) = second.picture(stream["generation"].as_u64().unwrap()).await;
        assert!(key);
        // Operating makes the second viewer decide the size and the screen.
        second.click(&tab, 100.0, 25.0);
        for view in [&mut first, &mut second] {
            let stream = view
                .until("a 1000-wide stream", |message| {
                    message["type"] == "stream" && message["width"] == 1000
                })
                .await;
            assert_eq!(stream["height"], 700);
        }
        let screen = evaluate(
            &fixture,
            &tab,
            "[devicePixelRatio, screen.width, innerWidth, innerHeight]",
        )
        .await;
        assert_eq!(screen, json!([1, 1920, 1000, 700]));
        // It stays so when the second viewer leaves, until the first operates.
        assert_eq!(second.close().await.exit_code, 0);
        first.click(&tab, 100.0, 25.0);
        first
            .until("a 1600-wide stream", |message| {
                message["type"] == "stream" && message["width"] == 1600
            })
            .await;
        assert_eq!(first.close().await.exit_code, 0);
        fixture
    })
    .await;
}

/// The picture the viewer received, decoded by ffmpeg into pixels.
fn decoded(frame: &[u8]) -> (u32, Vec<u8>) {
    let directory = tempfile::tempdir().expect("a place for the frame");
    let encoded = directory.path().join("frame.h264");
    let image = directory.path().join("frame.png");
    std::fs::write(&encoded, frame).expect("write the frame");
    let ffmpeg = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "h264", "-i"])
        .arg(&encoded)
        .args(["-frames:v", "1"])
        .arg(&image)
        .status()
        .expect("ffmpeg decodes the picture");
    assert!(ffmpeg.success(), "ffmpeg could not decode the picture");
    let bytes = std::fs::read(&image).expect("the decoded picture");
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .expect("png");
    let mut pixels = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut pixels).expect("png frame");
    pixels.truncate(info.buffer_size());
    assert_eq!(info.color_type, png::ColorType::Rgb, "ffmpeg writes RGB");
    (info.width, pixels)
}

/// The largest brightness step between neighbouring pixels of a row.
fn contrast(width: u32, pixels: &[u8], row: u32, from: u32, to: u32) -> u8 {
    let stride = width as usize * 3;
    let mut most = 0;
    for x in from..to.saturating_sub(1) {
        let at = row as usize * stride + x as usize * 3;
        if at + 4 >= pixels.len() {
            break;
        }
        most = most.max(pixels[at].abs_diff(pixels[at + 3]));
    }
    most
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_watched_tab_arrives_with_the_detail_of_the_viewers_ratio() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let tab = fixture
            .call("browser.open", json!({"url": site.base, "timeout": 120000}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut view = View::open(&fixture);
        // Half-CSS-pixel stripes are flat grey at ratio 1 and sharp above it.
        view.send(json!({"type": "hello", "platform": "linux"}));
        view.send(json!({
            "type": "panel", "width": 800, "height": 600, "devicePixelRatio": 1,
            "screenWidth": 1440, "screenHeight": 900,
        }));
        view.message("state").await;
        view.send(json!({"type": "watch", "tab": tab}));
        let single = view
            .until("a stream at ratio 1", |message| {
                message["type"] == "stream" && message["width"] == 800
            })
            .await;
        eventually(&fixture, &tab, "devicePixelRatio === 1").await;
        let (_, key, width, _, frame) = view.picture(single["generation"].as_u64().unwrap()).await;
        assert!(key);
        assert_eq!(width, 800);
        let (decoded_width, pixels) = decoded(&frame);
        assert_eq!(decoded_width, 800, "the picture is the viewport at ratio 1");
        let flat = contrast(decoded_width, &pixels, 400, 150, 350);

        // The same page at the viewer's ratio 2, where every stripe is a pixel.
        view.send(json!({
            "type": "panel", "width": 800, "height": 600, "devicePixelRatio": 2,
            "screenWidth": 1440, "screenHeight": 900,
        }));
        let double = view
            .until("a stream at ratio 2", |message| {
                message["type"] == "stream" && message["width"] == 1600
            })
            .await;
        eventually(&fixture, &tab, "devicePixelRatio === 2").await;
        let (_, key, width, height, frame) =
            view.picture(double["generation"].as_u64().unwrap()).await;
        assert!(key);
        assert_eq!((width, height), (1600, 1200));
        let (retina, pixels) = decoded(&frame);
        assert_eq!(retina, 1600, "the picture is twice the viewport at ratio 2");
        let sharp = contrast(retina, &pixels, 800, 300, 700);
        // At ratio 1 the stripes average into grey; at ratio 2 each one is its
        // own pixel, as sharp as the gradient's own antialiasing allows.
        assert!(flat < 40, "stripes at ratio 1 are flat grey, not {flat}");
        assert!(
            sharp > 100 && u16::from(sharp) > u16::from(flat) * 3,
            "stripes at ratio 2 are sharp, not {sharp} against {flat}",
        );

        // The agent's screenshot stays in CSS pixels whatever the ratio.
        let shot = fixture
            .call(
                "browser.screenshot",
                json!({"tab": tab, "output": "shot.png", "overwrite": true}),
            )
            .await;
        assert_eq!(shot["width"], 800);
        assert_eq!(shot["height"], 600);
        assert_eq!(shot["viewport"]["devicePixelRatio"], 2.0);
        assert_eq!(view.close().await.exit_code, 0);
        fixture
    })
    .await;
}
