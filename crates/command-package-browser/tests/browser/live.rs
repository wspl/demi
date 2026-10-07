//! The live view against real Chrome (`live-view.md` § Acceptance):
//! a viewer drives `browser.live` as the page does, beside the agent's
//! commands on the same tabs.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::families::{BrowserFixture, with_browser_fixture};
use axum::{
    Router,
    http::{HeaderMap, header::USER_AGENT},
    response::Html,
    routing::get,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use demi_command_package_browser_protocol::live::VideoHeader;
use demi_command_protocol::{CommandCaller, Completion, Record};
use demi_command_sdk::{Input, ServiceError};
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
        // A page that names its icon, one from its own site.
        .route(
            "/icon",
            get(|| async { Html(r#"<!doctype html><link rel="icon" href="/icon.svg"><title>Icon</title>"#) }),
        )
        .route(
            "/icon.svg",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="red"/></svg>"#,
                )
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
                            let (header, _) = VideoHeader::split(&frame).unwrap();
                            Frame::Video {
                                tab: header.tab.to_string(),
                                generation: header.generation,
                                sequence: header.sequence,
                                key: header.key,
                                width: header.width,
                                height: header.height,
                                data: frame.slice(VideoHeader::BYTES..),
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
        self.try_send(message).unwrap();
    }

    /// Sends viewer control, retaining closure for buffered-frame acknowledgements.
    fn try_send(
        &self,
        message: Value,
    ) -> Result<(), mpsc::error::SendError<Result<Bytes, ServiceError>>> {
        let json = serde_json::to_vec(&message).unwrap();
        let mut frame = BytesMut::new();
        frame.put_u32(1 + json.len() as u32);
        frame.put_u8(1);
        frame.put_slice(&json);
        self.input.as_ref().unwrap().send(Ok(frame.freeze()))
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
            // Closing the last tab ends input before already-buffered pictures
            // and the final `ended` control have drained. They need no ack.
            let _ended = self.try_send(json!({"type": "ack", "generation": generation, "sequence": sequence, "decodeQueue": 0}));
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

    /// The first video frame, following a replacement generation as the viewer does.
    async fn picture(&mut self, mut generation: u64) -> (String, bool, u16, u16, Bytes) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut seen = Vec::new();
        loop {
            let frame = tokio::time::timeout_at(deadline, self.next())
                .await
                .unwrap_or_else(|_| {
                    panic!("no picture for generation {generation}; controls: {seen:?}")
                });
            if let Frame::Control(message) = &frame
                && message["type"] != "heartbeat"
            {
                if message["type"] == "stream" {
                    generation = message["generation"].as_u64().unwrap();
                }
                seen.push(message.clone());
            }
            if let Frame::Video {
                tab,
                generation: arrived,
                key,
                width,
                height,
                data,
                ..
            } = frame
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

/// Polls the tab list until it lists `tab` as `wanted` says, given the
/// list's number and the tab's row.
async fn listed_until(fixture: &BrowserFixture, tab: &str, wanted: impl Fn(u64, &Value) -> bool) {
    let mut last = Value::Null;
    for _ in 0..200 {
        let (_, tabs) = fixture
            .result_for(user(), "browser.tabs", json!({}), CancellationToken::new(), Vec::new())
            .await;
        let row = tabs["tabs"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["id"] == json!(tab)));
        if let (Some(list), Some(row)) = (tabs["list"].as_u64(), row)
            && wanted(list, row)
        {
            return;
        }
        last = tabs;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never listed as wanted: {last}");
}

/// A user's navigation of `tab` to `url` answered at once, with the number
/// of the last tab list before it started.
fn moved(answer: &Value, tab: &str, url: &str) -> u64 {
    assert_eq!((&answer["tab"], &answer["url"]), (&json!(tab), &json!(url)), "{answer}");
    answer["list"].as_u64().expect("a user's navigation names its tab list")
}

/// The caller of the page's view and requests.
fn user() -> CommandCaller {
    serde_json::from_value(json!({"kind": "user"})).unwrap()
}

/// The exit code and answer of `operation` run as the page's request runs
/// it: for a `user` caller (`live-view.md` § The tab methods).
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

/// Watches `tab`, whose capture starts at the panel's size: the first
/// stream is the panel's, never one the module is about to replace
/// (`live-view.md` § Delivery).
async fn watch(view: &mut View, tab: &str) -> u64 {
    view.send(json!({"type": "watch", "tab": tab}));
    let stream = view.message("stream").await;
    assert_eq!(
        (&stream["tab"], &stream["width"], &stream["height"]),
        (&json!(tab), &json!(1600), &json!(1200)),
        "the first stream has the panel's size: {stream}"
    );
    assert_eq!(
        (&stream["viewport"], &stream["scale"]),
        (
            &json!({"width": 800, "height": 600, "devicePixelRatio": 2.0, "mode": "web"}),
            &json!(1.0)
        ),
        "a stream names the viewport its pictures show"
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

/// The view resolves the page's cursor locally (`live-view.md` § Input):
/// the observer of every frame, a cross-site frame in its own process
/// included, reports where each cursor applies, placed in tab CSS pixels; a
/// shadow root's elements count; a cursor list counts by its last keyword;
/// what a scrolled container hides is left out; and a new document resolves
/// its cursor at the still pointer.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn the_view_hears_where_each_cursor_applies_in_every_frame() {
    with_browser_fixture(|fixture| async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // `localhost` is another site than `127.0.0.1`: its frame runs in a process of its own.
        let page = format!(
            "<!doctype html><style>body{{margin:0;font:16px sans-serif}}</style>\
            <button style=\"position:fixed;inset:0;visibility:hidden;opacity:0;cursor:pointer\"></button>\
            <button style=\"position:absolute;left:10px;top:10px;width:100px;height:30px;cursor:pointer\"><span>Save</span></button>\
            <div style=\"position:absolute;left:10px;top:400px;pointer-events:none;cursor:wait\">\
              <span style=\"pointer-events:auto;display:inline-block;width:40px;height:20px\"></span></div>\
            <div style=\"position:absolute;left:10px;top:50px;width:100px;height:30px;cursor:url(data:image/gif;base64,R0lGODlhAQABAAAAACw=) 4 4, move\"></div>\
            <div style=\"position:absolute;left:10px;top:100px;width:100px;height:50px;overflow:auto\">\
              <div style=\"height:200px\"><div style=\"margin-top:120px;height:20px;cursor:help\"></div></div></div>\
            <iframe style=\"position:absolute;left:200px;top:100px;width:200px;height:100px;border:5px solid black\"\
              srcdoc=\"<body style='margin:0'><div style='margin:10px;width:50px;height:20px;cursor:crosshair'></div><p style='margin:0 10px'>Frame text</p></body>\"></iframe>\
            <iframe style=\"position:absolute;left:450px;top:100px;width:200px;height:100px;border:3px solid black\" src=\"http://localhost:{port}/frame\"></iframe>\
            <div id=\"host\" style=\"position:absolute;left:10px;top:300px\"></div>\
            <div id=\"sealed\" style=\"position:absolute;left:10px;top:340px;cursor:progress\"></div>\
            <script>host.attachShadow({{mode:'open'}}).innerHTML = '<button style=\"width:80px;height:20px;cursor:copy\">Shadow</button>';\
              sealed.attachShadow({{mode:'closed'}}).innerHTML = '<button style=\"width:80px;height:20px;cursor:zoom-in\">Closed</button>'</script>\
            <p style=\"position:absolute;left:10px;top:220px;margin:0\">Some text</p>"
        );
        let frame = "<!doctype html><body style=\"margin:0\"><button style=\"margin:20px;width:60px;height:30px;cursor:cell\">Far</button></body>";
        let next = "<!doctype html><style>body{margin:0}</style><button style=\"position:absolute;left:0;top:0;width:300px;height:300px;cursor:grab\">Next</button>";
        let app = Router::new()
            .route("/", get(move || async move { Html(page) }))
            .route("/frame", get(move || async move { Html(frame) }))
            .route("/next", get(move || async move { Html(next) }));
        let _server = AbortOnDropHandle::new(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let tab = fixture
            .call("browser.open", json!({"url": format!("http://127.0.0.1:{port}/")}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut view = View::open(&fixture);
        hello(&view, "mac");
        view.message("state").await;
        view.send(json!({"type": "watch", "tab": tab}));
        let has = |regions: &Value, cursor: &str| {
            regions.as_array().is_some_and(|regions| regions.iter().any(|region| region["cursor"] == cursor))
        };
        let cursors = view
            .until("the page's and its frames' cursors", |message| {
                message["type"] == "cursors"
                    && ["pointer", "crosshair", "cell", "copy", "progress"].iter().all(|cursor| has(&message["regions"], cursor))
            })
            .await;
        let regions = cursors["regions"].as_array().unwrap();
        let region = |cursor: &str| {
            regions
                .iter()
                .find(|region| region["cursor"] == cursor)
                .map(|region| [&region["x"], &region["y"], &region["width"], &region["height"]].map(|value| value.as_f64().unwrap()))
        };
        // A hidden backdrop over the whole page, which the pointer passes
        // through, shows no cursor: only the button does.
        let pointers: Vec<_> = regions.iter().filter(|region| region["cursor"] == "pointer").collect();
        assert_eq!(pointers.len(), 1, "{pointers:?}");
        assert_eq!(region("pointer"), Some([10.0, 10.0, 100.0, 30.0]));
        // Inside an element the pointer passes through, a child it reaches
        // shows the cursor it inherits.
        assert_eq!(region("wait"), Some([10.0, 400.0, 40.0, 20.0]));
        // A cursor list falls back to its last keyword.
        assert_eq!(region("move"), Some([10.0, 50.0, 100.0, 30.0]));
        // A frame's region, in the tab's coordinates past its border.
        assert_eq!(region("crosshair"), Some([215.0, 115.0, 50.0, 20.0]));
        // A cross-site frame's, from its own process, placed the same way.
        assert_eq!(region("cell"), Some([473.0, 123.0, 60.0, 30.0]));
        // A button in a shadow root.
        assert_eq!(region("copy"), Some([10.0, 300.0, 80.0, 20.0]));
        // A closed shadow root, which no script outside it reaches, shows its
        // host's cursor.
        assert_eq!(region("progress").map(|[x, y, ..]| [x, y]), Some([10.0, 340.0]));
        assert_eq!(region("zoom-in"), None);
        // A region its scrolled container hides is left out.
        assert_eq!(region("help"), None);
        // The cursor the observer resolves at the pointer: text over text.
        view.pointer(&tab, "move", 20.0, 228.0);
        view.until("a text cursor", |message| message["type"] == "cursor" && message["cursor"] == "text").await;
        // The tab has one cursor whichever frame resolved it: back from a
        // frame's text, the top document's empty space shows its own again.
        let cursor = |cursor: &'static str| move |message: &Value| message["type"] == "cursor" && message["cursor"] == cursor;
        view.pointer(&tab, "move", 150.0, 450.0);
        view.until("the top document's default cursor", cursor("default")).await;
        view.pointer(&tab, "move", 220.0, 150.0);
        view.until("the frame's text cursor", cursor("text")).await;
        view.pointer(&tab, "move", 150.0, 450.0);
        view.until("the top document's default cursor again", cursor("default")).await;
        view.pointer(&tab, "move", 20.0, 228.0);
        view.until("the text cursor again", cursor("text")).await;
        // A new document under the still pointer resolves its own cursor there.
        fixture
            .call("browser.goto", json!({"tab": tab, "url": format!("http://127.0.0.1:{port}/next")}))
            .await;
        view.until("the new document's cursor", |message| message["type"] == "cursor" && message["cursor"] == "grab").await;
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
        // Controls may arrive before the first video stream. Do not discard them
        // while waiting for video; capture delivery is tested separately.
        view.send(json!({"type": "watch", "tab": tab}));
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
            json!({"type": "state", "running": false, "list": 0, "tabs": [], "watched": null})
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
        fixture
            .call(
                "browser.fill",
                json!({"tab": tab, "css": "#text", "text": "retained across capture recovery"}),
            )
            .await;
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
        assert_eq!(
            evaluate(&fixture, &tab, "document.querySelector('#text').value").await,
            "retained across capture recovery"
        );
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

/// The user closes a tab whose page is still loading, the only tab and one
/// beside another. About 13 s here: closing the only tab stops Chrome and the
/// next open starts another, 2.2 s a round in the container, where init
/// reaps Chrome's detached helper about once a second, so that round repeats
/// three times; it cannot race, since the registry seals the browser instead
/// of closing the page. A close beside another tab asks Chrome to close the
/// loading page, the race this test guards, and costs 0.2 s a round.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires pinned real Chrome for Testing"]
async fn the_user_can_immediately_close_the_first_loading_tab() {
    with_browser_fixture(|fixture| async move {
        let path = fixture.root.path().join("loading.html");
        std::fs::write(&path, include_str!("families/loading-form.html")).unwrap();
        let url = url::Url::from_file_path(&path).unwrap();
        for _ in 0..3 {
            let opened = request(&fixture, "browser.open", json!({"url": url.as_str()})).await;
            let tab = opened["tab"].as_str().unwrap();
            let closed = request(
                &fixture,
                "browser.close",
                json!({"tab": tab, "timeout": 3000}),
            )
            .await;
            assert_eq!(closed, json!({"closed": tab}));
        }
        let keeper = request(&fixture, "browser.open", json!({"url": "about:blank"})).await;
        for _ in 0..10 {
            let opened = request(&fixture, "browser.open", json!({"url": url.as_str()})).await;
            let tab = opened["tab"].as_str().unwrap();
            let closed = request(
                &fixture,
                "browser.close",
                json!({"tab": tab, "timeout": 3000}),
            )
            .await;
            assert_eq!(closed, json!({"closed": tab}));
        }
        request(&fixture, "browser.close", json!({"tab": keeper["tab"]})).await;
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
        // Each answer names the last list before the request; a later list,
        // numbered higher, tells where the tab's history then stands.
        let after = |list: u64, back: bool, forward: bool| {
            move |number: u64, row: &Value| {
                number > list
                    && row["loading"] == json!(false)
                    && row["canGoBack"] == json!(back)
                    && row["canGoForward"] == json!(forward)
            }
        };
        let went = request(
            &fixture,
            "browser.goto",
            json!({"tab": tab, "url": site.base}),
        )
        .await;
        let list = moved(&went, &tab, &site.base);
        eventually(&fixture, &tab, &at(&site.base)).await;
        listed_until(&fixture, &tab, after(list, true, false)).await;
        let went = request(&fixture, "browser.goto", json!({"tab": tab, "url": second})).await;
        let list = moved(&went, &tab, &second);
        eventually(&fixture, &tab, &at(&second)).await;
        listed_until(&fixture, &tab, after(list, true, false)).await;
        let back = request(&fixture, "browser.back", json!({"tab": tab})).await;
        let list = moved(&back, &tab, &site.base);
        eventually(&fixture, &tab, &at(&site.base)).await;
        listed_until(&fixture, &tab, after(list, true, true)).await;
        let forward = request(&fixture, "browser.forward", json!({"tab": tab})).await;
        let list = moved(&forward, &tab, &second);
        eventually(&fixture, &tab, &at(&second)).await;
        listed_until(&fixture, &tab, after(list, true, false)).await;
        let (code, refused) = user_result(&fixture, "browser.forward", json!({"tab": tab})).await;
        assert_eq!(
            (code, &refused["error"]["code"]),
            (1, &json!("history_boundary"))
        );
        let origin = evaluate(&fixture, &tab, "performance.timeOrigin").await;
        let reloaded = request(&fixture, "browser.reload", json!({"tab": tab})).await;
        let list = moved(&reloaded, &tab, &second);
        // A new document starts its own clock.
        let renewed =
            format!("performance.timeOrigin > {origin} && document.readyState === 'complete'");
        eventually(&fixture, &tab, &renewed).await;
        listed_until(&fixture, &tab, after(list, true, false)).await;

        let started = Instant::now();
        let went = request(&fixture, "browser.goto", json!({"tab": tab, "url": slow})).await;
        assert!(started.elapsed() < prompt, "goto waited for its page");
        moved(&went, &tab, &slow);
        // The user's Stop ends the load, which would otherwise last the test
        // out: a list numbered after it says the tab no longer loads.
        listed_until(&fixture, &tab, |_, row| row["loading"] == json!(true)).await;
        let stopped = request(&fixture, "browser.stop", json!({"tab": tab})).await;
        let list = stopped["list"].as_u64().expect("Stop names its tab list");
        listed_until(&fixture, &tab, |number, row| {
            number > list && row["loading"] == json!(false)
        })
        .await;
        let last = request(&fixture, "browser.tabs", json!({})).await["list"]
            .as_u64()
            .expect("the user's tab list names its number");

        let missing = "t999";
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
        // Both were closed on purpose, the last one with its browser, so the
        // work panel removes their tabs rather than open them again.
        assert_eq!(tabs["closed"], json!([other, tab]));
        // With no browser running the list keeps the conversation's number,
        // and the next browser numbers its lists after the last browser's, so a
        // request the old one answered ends on the new one's first list.
        assert!(tabs["list"].as_u64().unwrap() >= last, "{tabs} went below {last}");
        let reopened = request(&fixture, "browser.open", json!({"url": "about:blank"})).await;
        let listed = request(&fixture, "browser.tabs", json!({})).await;
        assert!(
            listed["list"].as_u64().unwrap() > last,
            "the new browser's list {listed} is not numbered after {last}"
        );
        request(&fixture, "browser.close", json!({"tab": reopened["tab"]})).await;
        fixture
    })
    .await;
}

/// A tab lists its page's icon as a web browser's tab shows it, and a page
/// without one, whose site has no `/favicon.ico` either, lists none. Costs
/// a browser start and two page loads, about 2 s.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_tab_lists_its_pages_icon() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let icon = format!("{}icon", site.base);
        let opened = request(&fixture, "browser.open", json!({"url": icon})).await;
        let tab = opened["tab"].as_str().unwrap().to_owned();
        listed_until(&fixture, &tab, |_, row| {
            row["favicon"]
                .as_str()
                .is_some_and(|url| url.starts_with("data:image/png;base64,"))
        })
        .await;
        let went = request(&fixture, "browser.goto", json!({"tab": tab, "url": site.base})).await;
        let list = moved(&went, &tab, &site.base);
        listed_until(&fixture, &tab, |number, row| {
            number > list && row["loading"] == json!(false) && row.get("favicon").is_none()
        })
        .await;
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
        // The page's pixel ratio settles a moment after the new picture starts.
        eventually(
            &fixture,
            &tab,
            "devicePixelRatio === 1 && screen.width === 1920 && innerWidth === 1000 && innerHeight === 700",
        )
        .await;
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

/// Decodes a key frame as the page does (`pictures.ts`: WebCodecs, with the
/// live protocol's codec) and keeps the picture in the page as a PNG in
/// base64, the form in which a picture leaves Chrome whole; answers its
/// length.
const DECODE_PICTURE: &str = r#"async (encoded, codec) => {
  const data = Uint8Array.from(atob(encoded), (character) => character.charCodeAt(0));
  let decoder;
  const picture = await new Promise((resolve, reject) => {
    decoder = new VideoDecoder({ output: resolve, error: reject });
    decoder.configure({ codec, optimizeForLatency: true });
    decoder.decode(new EncodedVideoChunk({ type: 'key', timestamp: 0, data }));
    // Every picture is out before the flush resolves.
    decoder.flush().then(() => reject(new Error('the frame decoded to no picture')), reject);
  });
  decoder.close();
  const canvas = new OffscreenCanvas(picture.displayWidth, picture.displayHeight);
  canvas.getContext('2d', { alpha: false }).drawImage(picture, 0, 0);
  picture.close();
  const png = await canvas.convertToBlob({ type: 'image/png' });
  const url = await new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(png);
  });
  globalThis.decodedPicture = url.slice(url.indexOf(',') + 1);
  return globalThis.decodedPicture.length;
}"#;

/// How much of a kept picture one answer carries, within a command's 64 KiB
/// output.
const PICTURE_SLICE: usize = 48 * 1024;

/// Runs `expression` in `tab` through the agent's CDP command, which, unlike
/// `browser.eval`, allows side effects and awaits a promise, and answers its
/// value.
async fn run_in_page(fixture: &BrowserFixture, tab: &str, expression: &str) -> Value {
    let params = json!({"expression": expression, "awaitPromise": true, "returnByValue": true});
    let answer = fixture
        .call(
            "browser.cdp.send",
            json!({"tab": tab, "method": "Runtime.evaluate", "params": params.to_string()}),
        )
        .await;
    assert!(
        answer["result"].get("exceptionDetails").is_none(),
        "{expression:.80}: {answer}"
    );
    answer["result"]["result"]["value"].clone()
}

/// The picture the viewer received, decoded by the Chrome that shows `tab`
/// as the page decodes it, into RGB pixels.
async fn decoded(fixture: &BrowserFixture, tab: &str, frame: &[u8]) -> (u32, Vec<u8>) {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    let decode = format!(
        "({DECODE_PICTURE})('{}', '{}')",
        STANDARD.encode(frame),
        demi_command_package_browser_protocol::live::VIDEO_CODEC
    );
    let length = run_in_page(fixture, tab, &decode)
        .await
        .as_u64()
        .expect("the kept picture's length") as usize;
    let mut encoded = String::with_capacity(length);
    while encoded.len() < length {
        let from = encoded.len();
        let slice = format!(
            "globalThis.decodedPicture.slice({from}, {})",
            from + PICTURE_SLICE
        );
        let value = run_in_page(fixture, tab, &slice).await;
        encoded.push_str(value.as_str().expect("a slice of the kept picture"));
    }
    run_in_page(fixture, tab, "delete globalThis.decodedPicture").await;
    let bytes = STANDARD.decode(encoded).expect("the picture in base64");
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .expect("png");
    let mut pixels = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut pixels).expect("png frame");
    pixels.truncate(info.buffer_size());
    let pixels = match info.color_type {
        png::ColorType::Rgb => pixels,
        png::ColorType::Rgba => pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|pixel| &pixel[..3])
            .copied()
            .collect(),
        other => panic!("a canvas's PNG is RGB or RGBA, not {other:?}"),
    };
    (info.width, pixels)
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_narrow_still_picture_matches_the_page_coordinates() {
    with_browser_fixture(|fixture| async move {
        let path = fixture.root.path().join("still.html");
        std::fs::write(&path, "<!doctype html><style>body{margin:0;background:white}</style><div style=\"position:fixed;left:10px;top:100px;width:100px;height:30px;background:red\"></div>").unwrap();
        let tab = fixture
            .call("browser.open", json!({"url": url::Url::from_file_path(path).unwrap().as_str()}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut view = View::open(&fixture);
        view.send(json!({"type": "hello", "platform": "mac"}));
        view.message("state").await;
        view.send(json!({"type": "watch", "tab": tab}));
        // Each size starts a capture whose stream names its pictures' size.
        // Chrome captures an odd side at ratio 1 at the nearest even size of
        // the page's shape, so both sides may lose a pixel: 537 × 912 came
        // as 536 × 910.
        for (width, height, ratio) in [(409, 632, 2), (800, 600, 2), (500, 400, 2), (409, 632, 2), (537, 912, 1)] {
            view.send(json!({
                "type": "panel", "width": width, "height": height, "devicePixelRatio": ratio,
                "screenWidth": 1920, "screenHeight": 1080,
            }));
            let stream = view
                .until("a resized stream", |message| {
                    message["type"] == "stream"
                        && message["viewport"]["width"] == width
                        && message["viewport"]["height"] == height
                        && message["viewport"]["devicePixelRatio"].as_f64() == Some(f64::from(ratio))
                })
                .await;
            let pixels_wide = stream["width"].as_u64().unwrap() as u32;
            let pixels_high = stream["height"].as_u64().unwrap() as u32;
            let (wide, high): (u32, u32) = (width * ratio, height * ratio);
            assert!(
                pixels_wide % 2 == 0 && pixels_high % 2 == 0 && wide.abs_diff(pixels_wide) <= 2 && high.abs_diff(pixels_high) <= 2,
                "the stream of {wide} × {high} pixels is {pixels_wide} × {pixels_high}",
            );
            let (_, key, frame_wide, frame_high, frame) = view.picture(stream["generation"].as_u64().unwrap()).await;
            assert!(key);
            assert_eq!((u32::from(frame_wide), u32::from(frame_high)), (pixels_wide, pixels_high), "the stream names its pictures' size");
            let (decoded_width, pixels) = decoded(&fixture, &tab, &frame).await;
            assert_eq!(decoded_width, pixels_wide);
            let rgb = |x: u32, y: u32| {
                let at = ((y * decoded_width + x) * 3) as usize;
                [pixels[at], pixels[at + 1], pixels[at + 2]]
            };
            let white = |[red, green, blue]: [u8; 3]| red > 200 && green > 200 && blue > 200;
            // Red against white differs most in green, which an edge's compression blurs least.
            let red = |[red, green, _]: [u8; 3]| red > 150 && green < 80;

            for (x, y) in [(4, 4), (pixels_wide - 5, pixels_high - 5)] {
                assert!(white(rgb(x, y)), "white page corner at {x},{y}: {:?}", rgb(x, y));
            }
            // The rectangle's edges, at CSS 10 and 110 across, fall on the
            // page's own device pixels when the sides are even, and within
            // the pixel Chrome's scaling blends when one is odd.
            let at = |css: u32| f64::from(css * ratio) * f64::from(pixels_wide) / f64::from(wide);
            let row = 115 * ratio;
            let left = at(10);
            let right = at(110);
            let edges = [
                (left.floor() as u32 - 1, false),
                (left.ceil() as u32, true),
                (right.floor() as u32 - 1, true),
                (right.ceil() as u32, false),
            ];
            for (x, inside) in edges {
                let pixel = rgb(x, row);
                assert!(if inside { red(pixel) } else { white(pixel) }, "the rectangle's edge at {x}, ratio {ratio}: {pixel:?}");
            }
        }
        view.close().await;
        fixture
    })
    .await;
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
        let (decoded_width, pixels) = decoded(&fixture, &tab, &frame).await;
        assert_eq!(decoded_width, 800, "the picture is the viewport at ratio 1");
        // Sample above the moving red marker, which crosses the lower stripes.
        let flat = contrast(decoded_width, &pixels, 390, 150, 350);

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
        let (retina, pixels) = decoded(&fixture, &tab, &frame).await;
        assert_eq!(retina, 1600, "the picture is twice the viewport at ratio 2");
        let sharp = contrast(retina, &pixels, 780, 300, 700);
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

/// A frame from another site shows whole at every ratio the viewer moves
/// between, falling ratios included (`live-view.md` § Pixel ratio).
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_cross_site_frame_follows_the_viewers_new_ratio() {
    with_browser_fixture(|fixture| async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // `localhost` is another site than `127.0.0.1`: its frame runs in a
        // process of its own, which paints all of it green.
        let page = format!(
            "<!doctype html><style>body{{margin:0;background:white}}</style>\
            <iframe style=\"position:absolute;left:20px;top:20px;width:200px;height:100px;border:0\" src=\"http://localhost:{port}/frame\"></iframe>"
        );
        let frame = "<!doctype html><body style=\"margin:0;background:#00ff00\"></body>";
        let app = Router::new()
            .route("/", get(move || async move { Html(page) }))
            .route("/frame", get(move || async move { Html(frame) }));
        let _server = AbortOnDropHandle::new(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        // The page loads while the viewer watches at ratio 2, as a page the
        // user opens in the panel does: its frame starts at that ratio.
        let tab = fixture
            .call("browser.open", json!({"url": "about:blank"}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut view = View::open(&fixture);
        hello(&view, "mac");
        view.message("state").await;
        view.send(json!({"type": "watch", "tab": tab}));
        let panel = |width: u32, ratio: u32| {
            json!({
                "type": "panel", "width": width, "height": 912, "devicePixelRatio": ratio,
                "screenWidth": 800, "screenHeight": 600,
            })
        };
        view.send(panel(400, 2));
        view.until("a stream at ratio 2", |message| {
            message["type"] == "stream" && message["width"] == 800
        })
        .await;
        fixture
            .call("browser.goto", json!({"tab": tab, "url": format!("http://127.0.0.1:{port}/")}))
            .await;
        // The viewer's ratio goes to 1 and back, as when the window moves
        // between a Retina screen and another, and the panel's size changes
        // with it.
        for (width, ratio) in [(358u32, 1u32), (400, 2)] {
            view.send(panel(width, ratio));
            let stream = view
                .until("a stream at the new ratio", |message| {
                    message["type"] == "stream" && message["width"] == width * ratio
                })
                .await;
            let (_, key, _, _, picture) = view.picture(stream["generation"].as_u64().unwrap()).await;
            assert!(key);
            let (decoded_width, pixels) = decoded(&fixture, &tab, &picture).await;
            assert_eq!(decoded_width, width * ratio);
            let rgb = |x: u32, y: u32| {
                let at = ((y * decoded_width + x) * 3) as usize;
                [pixels[at], pixels[at + 1], pixels[at + 2]]
            };
            let green = |[red, green, blue]: [u8; 3]| red < 80 && green > 200 && blue < 80;
            // The frame's corners, inside its 200 x 100 CSS pixels.
            for (x, y) in [(25, 25), (215, 25), (25, 115), (215, 115)] {
                let pixel = rgb(x * ratio, y * ratio);
                assert!(green(pixel), "the frame at CSS {x},{y}, ratio {ratio}: {pixel:?}");
            }
        }
        view.close().await;
        fixture
    })
    .await;
}

/// The capture extension runs beside the pages as soon as the browser does,
/// and the tab registry never lists it (`live-view.md` § Capture).
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn the_capture_extension_runs_beside_the_pages_and_is_never_a_tab() {
    crate::fixture::with_fixture(|environment, _| async move {
        let cancel = CancellationToken::new();
        let timeout = Duration::from_secs(60);
        let tab = environment.open("about:blank", &cancel, timeout).await?;
        let prefix = format!(
            "chrome-extension://{}/",
            demi_command_package_browser_chrome::driver::testing::CAPTURE_EXTENSION_ID
        );
        // The extension's worker starts on its own, some time after the browser.
        tokio::time::timeout(timeout, async {
            while !environment
                .targets()
                .await?
                .iter()
                .any(|target| target.url.starts_with(&prefix))
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok::<_, demi_command_package_browser_chrome::driver::operation::BrowserError>(())
        })
        .await
        .expect("the capture extension runs")?;
        let tabs = environment.tabs(&cancel, timeout).await?;
        let ids: Vec<_> = tabs.iter().map(|listed| listed.id().clone()).collect();
        assert_eq!(ids, vec![tab.id().clone()]);
        Ok(())
    })
    .await;
}

/// A reload of the capture extension, as Chrome may do, keeps the pages as
/// they were and brings its worker back each time (`live-view.md` § Capture).
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn capture_extension_reload_preserves_pages_and_recreates_its_worker() {
    crate::fixture::with_fixture(|environment, _| async move {
        let cancel = CancellationToken::new();
        let timeout = Duration::from_secs(30);
        let tab = environment.open("about:blank", &cancel, timeout).await?;
        let worker_url = format!(
            "chrome-extension://{}/background.js",
            demi_command_package_browser_chrome::driver::testing::CAPTURE_EXTENSION_ID
        );
        let offscreen_url = worker_url.replace("background.js", "offscreen.html");
        let mut previous = None;
        for round in 0..4 {
            let worker = tokio::time::timeout(timeout, async {
                loop {
                    let targets = environment.targets().await?;
                    // Target discovery precedes the worker's start: the
                    // offscreen document shows its startup code has run.
                    let started = targets.iter().any(|target| target.url == offscreen_url);
                    let worker = targets.into_iter().find(|target| {
                        target.url == worker_url && previous.as_ref() != Some(&target.target_id)
                    });
                    if let Some(worker) = worker.filter(|_| started) {
                        return Ok::<
                            _,
                            demi_command_package_browser_chrome::driver::operation::BrowserError,
                        >(worker.target_id);
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .expect("the capture worker returns after every reload")?;
            assert_eq!(environment.tabs(&cancel, timeout).await?.len(), 1);
            assert_eq!(
                demi_command_package_browser_chrome::page::evaluation::evaluate(
                    &tab,
                    "document.URL",
                    &cancel,
                    timeout
                )
                .await?,
                json!("about:blank")
            );
            if round == 3 {
                break;
            }
            let reloading = demi_command_package_browser_chrome::cdp::testing::evaluate_in(
                &environment,
                worker.clone(),
                "setTimeout(() => chrome.runtime.reload(), 100); true",
            )
            .await?;
            assert_eq!(reloading, json!(true));
            previous = Some(worker);
        }
        Ok(())
    })
    .await;
}

/// A page with a link, a field, an element whose own menu the page shows, a
/// download and a select, for the browser's menu, the user's downloads and a
/// page Back brings back.
const MENU_PAGE: &str = r#"<!doctype html>
<style>body { margin: 0; font: 16px sans-serif }</style>
<a id="link" href="/two" style="position: absolute; left: 10px; top: 10px">Page two</a>
<input id="field" style="position: absolute; left: 10px; top: 60px; width: 200px; height: 30px">
<div id="own" oncontextmenu="event.preventDefault()"
  style="position: absolute; left: 10px; top: 110px; width: 200px; height: 30px">Own menu</div>
<a id="download" href="/report" style="position: absolute; left: 10px; top: 160px">Report</a>
<select id="choice" style="position: absolute; left: 10px; top: 210px; width: 200px; height: 30px">
  <option>A</option><option>B</option>
</select>
<p id="words" style="position: absolute; left: 10px; top: 250px; margin: 0">Some words</p>"#;

async fn menu_site() -> (String, AbortOnDropHandle<()>) {
    let app = Router::new()
        .route("/", get(|| async { Html(MENU_PAGE) }))
        .route("/two", get(|| async { Html("<!doctype html><p>Two</p>") }))
        .route(
            "/report",
            get(|| async {
                (
                    [
                        (axum::http::header::CONTENT_TYPE, "text/plain"),
                        (
                            axum::http::header::CONTENT_DISPOSITION,
                            "attachment; filename=\"report.txt\"",
                        ),
                    ],
                    "the report",
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let task = AbortOnDropHandle::new(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    (base, task)
}

impl View {
    /// The viewer's right click, which the page may leave to the browser.
    fn right_click(&self, tab: &str, x: f64, y: f64) {
        for (action, buttons) in [("down", 2), ("up", 0)] {
            self.send(json!({
                "type": "pointer", "tab": tab, "action": action, "x": x, "y": y,
                "button": "right", "buttons": buttons, "clickCount": 1, "modifiers": 0,
            }));
        }
    }
}

/// The user's right clicks open the browser's menu with what lies under
/// them, unless the page shows its own; the user's downloads are named after
/// their files in the browser's download directory; and a page Back brings
/// back from the browser's cache reports its controls again; a right click
/// on a link selects nothing. About 4 s.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn the_users_menus_downloads_and_returning_pages_reach_the_viewer() {
    with_browser_fixture(|fixture| async move {
        let (base, _site) = menu_site().await;
        let tab = fixture
            .call("browser.open", json!({"url": base, "timeout": 120000}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut view = View::open(&fixture);
        hello(&view, "linux");
        view.message("state").await;
        view.send(json!({"type": "watch", "tab": tab}));
        view.until("the page's select", |message| {
            message["type"] == "controls" && message["controls"].as_array().is_some_and(|controls| controls.len() == 1)
        })
        .await;

        // A link: the menu names its address, and the right click selected
        // nothing, though Chrome on a Mac Host would select the link.
        view.right_click(&tab, 20.0, 15.0);
        let menu = view.message("menu").await;
        assert_eq!(menu["tab"], json!(tab));
        assert_eq!(menu["menu"]["link"], json!(format!("{base}two")));
        assert_eq!(menu["menu"]["editable"], json!(false));
        assert_eq!(menu["menu"]["selection"], json!(false), "{menu}");
        // The page's own menu is the page's: the next menu is the field's.
        view.right_click(&tab, 50.0, 125.0);
        view.right_click(&tab, 50.0, 75.0);
        let menu = view.message("menu").await;
        assert_eq!(menu["menu"]["y"], json!(75.0), "{menu}");
        assert_eq!((&menu["menu"]["editable"], &menu["menu"]["link"]), (&json!(true), &json!("")));
        // Selected text: the menu offers to copy it.
        fixture
            .call("browser.select-text", json!({"tab": tab, "css": "#words", "text": "Some words"}))
            .await;
        view.right_click(&tab, 30.0, 258.0);
        let menu = view.message("menu").await;
        assert_eq!((&menu["menu"]["selection"], &menu["menu"]["link"]), (&json!(true), &json!("")));

        // Two downloads of one name: the second is numbered, both complete on the Host.
        view.click(&tab, 20.0, 165.0);
        view.until("the first download complete", |message| {
            message["type"] == "downloads" && message["downloads"][0]["state"] == "complete"
        })
        .await;
        view.click(&tab, 20.0, 165.0);
        let downloads = view
            .until("the second download complete", |message| {
                message["type"] == "downloads" && message["downloads"][1]["state"] == "complete"
            })
            .await;
        let names: Vec<&Value> = downloads["downloads"].as_array().unwrap().iter().map(|download| &download["name"]).collect();
        assert_eq!(names, [&json!("report.txt"), &json!("report.txt")]);
        let first = downloads["downloads"][0]["path"].as_str().unwrap();
        let second = downloads["downloads"][1]["path"].as_str().unwrap();
        assert!(first.ends_with("/report.txt"), "{first}");
        assert!(second.ends_with("/report (1).txt"), "{second}");
        for path in [first, second] {
            assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), "the report");
        }
        assert_eq!(downloads["downloads"][1]["total"], json!(10));

        // Away and Back: the page returns with its select.
        request(&fixture, "browser.goto", json!({"tab": tab, "url": format!("{base}two")})).await;
        view.until("the second page's no controls", |message| {
            message["type"] == "controls" && message["controls"] == json!([])
        })
        .await;
        request(&fixture, "browser.back", json!({"tab": tab})).await;
        view.until("the select again", |message| {
            message["type"] == "controls" && message["controls"].as_array().is_some_and(|controls| controls.len() == 1)
        })
        .await;
        assert_eq!(view.close().await.exit_code, 0);
        fixture
    })
    .await;
}
