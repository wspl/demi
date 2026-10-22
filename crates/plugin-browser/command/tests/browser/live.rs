//! The live view against real Chrome:
//! a viewer drives `browser.live` as the page does, beside the agent's
//! commands on the same tabs.

use std::time::Duration;

use crate::families::{BrowserFixture, with_browser_fixture};
use axum::{
    Router,
    response::Html,
    routing::get,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use demi_live_view_protocol::testing::Viewer;
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

/// A page with a mark in each corner of the viewport: red top left, green
/// top right, blue bottom left, black bottom right, on white.
const CORNERS: &str = r#"<!doctype html>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
html, body { margin: 0; overflow: hidden; background: #fff }
div { position: fixed; width: 40px; height: 40px }
</style>
<div style="left: 0; top: 0; background: #f00"></div>
<div style="right: 0; top: 0; background: #0f0"></div>
<div style="left: 0; bottom: 0; background: #00f"></div>
<div style="right: 0; bottom: 0; background: #000"></div>"#;

struct Site {
    base: String,
    _task: AbortOnDropHandle<()>,
}

async fn site() -> Site {
    let app = Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/corners", get(|| async { Html(CORNERS) }))
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
    Site { base, _task: task }
}

#[derive(Debug)]
enum Frame {
    Control(Value),
    Video {
        tab: String,
        generation: u32,
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
        // The page acknowledges each chunk as it arrives, while the view's
        // input lasts: a test that ends the input ends the stream.
        let acks = input.downgrade();
        let reading = tokio::spawn(async move {
            let mut pending = BytesMut::new();
            let mut viewer = Viewer::default();
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
                            let (ack, whole) = viewer.chunk(&frame).unwrap();
                            if let Some(acks) = acks.upgrade() {
                                // The view ended input before already-sent chunks drained; they need no acknowledgement then.
                                let _ended = acks.send(Ok(Bytes::from(ack)));
                            }
                            let Some((header, data)) = whole else {
                                continue;
                            };
                            Frame::Video {
                                tab: header.source.as_str().to_owned(),
                                generation: header.generation,
                                key: header.key,
                                width: header.width,
                                height: header.height,
                                data: Bytes::from(data),
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

    /// Sends the viewer's `input` to `tab`.
    fn input(&self, tab: &str, input: Value) {
        self.send(json!({"type": "input", "tab": tab, "input": input}));
    }

    fn raw(&self, bytes: Bytes) {
        self.input.as_ref().unwrap().send(Ok(bytes)).unwrap();
    }

    /// The next frame.
    async fn next(&mut self) -> Frame {
        tokio::time::timeout(Duration::from_secs(30), self.frames.recv())
            .await
            .expect("the module went quiet")
            .expect("the view ended")
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
        self.input(tab, json!({
            "type": "pointer", "action": action, "x": x, "y": y,
            "button": if action == "move" { "none" } else { "left" },
            "buttons": u8::from(pressed), "clickCount": u8::from(action != "move"), "modifiers": 0,
        }));
    }

    fn click(&self, tab: &str, x: f64, y: f64) {
        self.pointer(tab, "down", x, y);
        self.pointer(tab, "up", x, y);
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
/// it: for a `user` caller.
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
/// stream is the panel's, never one the module is about to replace.
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

/// About 3 s here: Chrome starts, one tab is watched at ratio 2 and
/// captured three times.
///
/// Defect this catches: Chrome's own scaling of a capture to CSS pixels,
/// while the live view captures the tab, answered only the top-left quarter
/// of the page.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_watched_tab_at_ratio_2_is_captured_whole_in_css_pixels() {
    with_browser_fixture(|fixture| async move {
        let site = site().await;
        let url = format!("{}corners", site.base);
        let tab = fixture
            .call("browser.open", json!({"url": url, "timeout": 120000}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut view = View::open(&fixture);
        hello(&view, "mac");
        view.message("state").await;
        watch(&mut view, &tab).await;
        // The view, the whole page and a rectangle across all four marks.
        for (args, name) in [
            (json!({}), "the view"),
            (json!({"full-page": true}), "the whole page"),
            (json!({"clip": "0,0,800,600"}), "a rectangle"),
        ] {
            let mut args = args;
            args["tab"] = json!(tab);
            args["output"] = json!(format!("{}.png", name.replace(' ', "-")));
            let shot = fixture.call("browser.screenshot", args).await;
            let bytes = std::fs::read(shot["path"].as_str().unwrap()).unwrap();
            let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut pixels).unwrap();
            assert_eq!((info.width, info.height), (800, 600), "{name}: one pixel per CSS pixel");
            let channels = info.color_type.samples();
            let corners = [(10, 10), (789, 10), (10, 589), (789, 589)].map(|(x, y)| {
                let at = (y * info.width as usize + x) * channels;
                [pixels[at], pixels[at + 1], pixels[at + 2]]
            });
            assert_eq!(
                corners,
                [[255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 0, 0]],
                "{name}: every corner's mark is in the picture"
            );
        }
        assert_eq!(view.close().await.exit_code, 0);
        fixture
    })
    .await;
}

/// The view resolves the page's cursor locally. The observer of every frame, a
/// cross-site frame in its own process included, reports where each cursor
/// applies, placed in tab CSS pixels.
///
/// - A shadow root's elements count.
/// - A cursor list counts by its last keyword.
/// - What a scrolled container hides is left out.
/// - A new document resolves its cursor at the still pointer.
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

/// The user closes a tab whose page is still loading: the only tab, and one
/// beside another.
///
/// The test takes about 13 s here.
///
/// - Closing the only tab stops Chrome and the next open starts another, 2.2 s
///   a round in the container, where init reaps Chrome's detached helper about
///   once a second, so that round repeats three times. It cannot race, since
///   the registry seals the browser instead of closing the page.
/// - A close beside another tab asks Chrome to close the loading page, the race
///   this test guards, and costs 0.2 s a round.
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

/// A panel that changes size while the watched tab loads an address that does
/// not answer leaves the tab at its size until the navigation ends.
///
/// Chrome holds the page's commands meanwhile but resizes its window at once,
/// so the capture letterboxed the page in black. The hub, waiting for the page,
/// left every viewer's request, a reloaded page's view included, unanswered
/// until Chrome gave up on the held command 30 s later.
///
/// Cost: about 2 s.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_panel_resized_while_a_navigation_hangs_keeps_the_picture_and_the_view_answers() {
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
        request(&fixture, "browser.goto", json!({"tab": tab, "url": format!("{}slow", site.base)})).await;
        view.until("the tab loading", |message| {
            message["type"] == "state" && message["tabs"][0]["loading"] == json!(true)
        })
        .await;
        view.send(json!({
            "type": "panel", "width": 700, "height": 500, "devicePixelRatio": 2,
            "screenWidth": 1440, "screenHeight": 900,
        }));
        // Another page's view opens and is answered while the page hangs:
        // far sooner than Chrome's 30 s for a held command, and in time for a
        // busy machine.
        let prompt = Duration::from_secs(10);
        let mut reloaded = View::open(&fixture);
        hello(&reloaded, "mac");
        tokio::time::timeout(prompt, reloaded.message("state"))
            .await
            .expect("the view answers a new viewer while the page hangs");
        reloaded.send(json!({"type": "watch", "tab": tab}));
        let stream = tokio::time::timeout(prompt, reloaded.message("stream"))
            .await
            .expect("the view answers a watch while the page hangs");
        assert_eq!(
            (stream["width"].as_u64(), stream["height"].as_u64()),
            (Some(1600), Some(1200)),
            "the tab keeps its size while its navigation is under way: {stream}"
        );
        assert_eq!(reloaded.close().await.exit_code, 0);
        // Once the navigation ends, the tab takes the panel's new size.
        request(&fixture, "browser.stop", json!({"tab": tab})).await;
        view.until("a 1400-wide stream", |message| {
            message["type"] == "stream" && message["width"] == 1400
        })
        .await;
        eventually(&fixture, &tab, "innerWidth === 700 && innerHeight === 500").await;
        assert_eq!(view.close().await.exit_code, 0);
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

/// How much of a kept picture one answer carries.
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
        demi_live_view_protocol::VIDEO_CODEC
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
        // The page notes each pixel ratio it is shown at.
        std::fs::write(&path, "<!doctype html><style>body{margin:0;background:white}</style><div style=\"position:fixed;left:10px;top:100px;width:100px;height:30px;background:red\"></div><script>window.ratios=[];(function watch(){ratios.push(devicePixelRatio);matchMedia(`(resolution: ${devicePixelRatio}dppx)`).addEventListener('change',watch,{once:true})})()</script>").unwrap();
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
        // The page is shown only at the ratios its viewer had. A capture that
        // outlived a resize raised it: 800 × 600 at 2 resized to 500 × 400
        // under the capture of 1600 × 1200 pixels came to 2.0000000596 every
        // time, and now and then a new capture's first picture showed it at 3.
        let ratios = evaluate(&fixture, &tab, "JSON.stringify(ratios)").await;
        let ratios: Vec<f64> = serde_json::from_str(ratios.as_str().unwrap()).unwrap();
        assert!(ratios.iter().all(|ratio| [1.0, 2.0].contains(ratio)), "the page's pixel ratios: {ratios:?}");
        view.close().await;
        fixture
    })
    .await;
}
