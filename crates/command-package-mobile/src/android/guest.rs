//! What Demi does inside an emulator over adb (`mobile.md` § Android): the
//! UI tree with its display rotation, a field's text and the clipboard
//! through openatx's `u2.jar`, which `demi-mobile` carries and runs with
//! `app_process` as a shell process, not an installed app; appearance and
//! recording through the shell.

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use bytes::Bytes;
use futures_util::future::BoxFuture;
use http_body_util::{BodyExt as _, Full};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::adb::Adb;
use crate::driver::{DriverError, RawElement, Rect, Result};

/// The port the helper listens on inside the emulator; another than the
/// 9008 uiautomator2's own clients use, so the two do not collide.
const PORT: u16 = 9018;
/// Where the helper's process id is kept, so the next program stops the
/// helper this one left.
const PID_FILE: &str = "/data/local/tmp/demi-u2.pid";
/// The recording's file while it runs.
const RECORDING: &str = "/data/local/tmp/demi-recording.mp4";
/// How long the helper takes to start.
const STARTING: Duration = Duration::from_secs(30);
/// What the helper prints once it listens.
const LISTENING: &str = "http server listening";
/// What the helper prints when another client holds the UI automation
/// connection, which Android lets one client have at a time.
const HELD: &str = "already registered";
/// The start of the helper's own `CLASSPATH`, which tells its process from
/// any other the id in a stale pid file may name now.
const OURS: &str = "CLASSPATH=/data/local/tmp/demi-u2-";

/// Why a command that needs the helper fails while another client, such as
/// Appium, holds the UI automation connection (`mobile.md` § Android).
pub const TAKEN: &str = "another UI automation client, such as Appium, holds this emulator's UI automation; \
                         Demi takes it back once that client lets it go";

#[cfg(demi_android_helper)]
const JAR: Option<(&[u8], &str)> = Some((include_bytes!(concat!(env!("OUT_DIR"), "/u2.jar")), env!("DEMI_U2_JAR_SHA256")));
#[cfg(not(demi_android_helper))]
const JAR: Option<(&[u8], &str)> = None;

/// What the driver does inside one emulator.
pub trait Guest: Send + Sync + 'static {
    /// The screen's tree, every window, as the helper dumps it: XML whose
    /// bounds are in Android's display pixels.
    fn hierarchy(&self) -> BoxFuture<'_, Result<String>>;
    /// The package of the input method, whose window is the software
    /// keyboard; none when it cannot be read.
    fn keyboard(&self) -> BoxFuture<'_, Option<String>>;
    /// Sets the focused field's text.
    fn set_text<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>>;
    /// The text on the clipboard, empty when it holds none.
    fn clipboard(&self) -> BoxFuture<'_, Result<String>>;
    fn set_clipboard<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>>;

    /// Whether the dark appearance is on.
    fn night(&self) -> BoxFuture<'_, Result<bool>>;
    fn set_night(&self, dark: bool) -> BoxFuture<'_, Result<()>>;
    fn record_start(&self) -> BoxFuture<'_, Result<()>>;
    /// Ends the recording and writes its MP4 to `file`.
    fn record_stop<'a>(&'a self, file: &'a Path) -> BoxFuture<'a, Result<()>>;
    /// Stops what Demi runs inside the emulator.
    fn close(&self) -> BoxFuture<'_, ()>;
}

/// The helper's process inside the emulator, while its shell connection
/// stays open.
struct Server {
    /// Cancelled once the process ended.
    ended: CancellationToken,
    /// Reads what the helper prints until it ends; aborted as Demi lets go.
    reader: tokio::task::JoinHandle<()>,
}

/// A recording running inside the emulator.
struct Recording {
    pid: u32,
    /// Ends with the recording's process.
    finished: tokio::task::JoinHandle<()>,
}

/// The emulator `serial`, reached through the adb server.
pub struct U2 {
    adb: Adb,
    serial: String,
    server: tokio::sync::Mutex<Option<Server>>,
    recording: Mutex<Option<Recording>>,
    /// The input method's package, read once.
    keyboard: tokio::sync::OnceCell<Option<String>>,
}

impl U2 {
    pub fn new(adb: Adb, serial: String) -> Arc<Self> {
        Arc::new(Self {
            adb,
            serial,
            server: tokio::sync::Mutex::default(),
            recording: Mutex::default(),
            keyboard: tokio::sync::OnceCell::new(),
        })
    }

    /// Starts the helper, copying it into the emulator first when it is not
    /// there, and stopping one an earlier program left.
    async fn start(&self) -> Result<Server> {
        let (jar, digest) = JAR.ok_or_else(|| {
            DriverError::Failed("this build of demi-mobile carries no Android helper".into())
        })?;
        let remote = format!("/data/local/tmp/demi-u2-{}.jar", &digest[..12]);
        let present = self.adb.shell(&self.serial, &format!("[ -f {remote} ] && echo present")).await?;
        if present.trim() != "present" {
            self.adb.push(&self.serial, jar, &remote, 0o644).await?;
        }
        self.stop_left().await?;
        let stream = self
            .adb
            .open(
                &self.serial,
                &format!(
                    "shell:echo $$ > {PID_FILE}; CLASSPATH={remote} exec app_process / com.wetest.uia2.Main -p {PORT} 2>&1"
                ),
            )
            .await?;
        let mut lines = BufReader::new(stream).lines();
        let mut printed = Vec::new();
        let listening = async {
            while let Some(line) = lines.next_line().await.transpose() {
                let line = line.map_err(|error| error.to_string())?;
                if line.contains(LISTENING) {
                    return Ok(());
                }
                // A helper Android refuses does not end: it waits on.
                if line.contains(HELD) {
                    return Err(HELD.to_owned());
                }
                printed.push(line);
            }
            Err(format!("it ended: {}", printed.join(" / ")))
        };
        match tokio::time::timeout(STARTING, listening).await {
            Ok(Ok(())) => {}
            Ok(Err(why)) if why == HELD => {
                drop(lines);
                if let Err(error) = self.stop_left().await {
                    tracing::info!("the refused Android helper did not stop: {error}");
                }
                return Err(DriverError::Failed(TAKEN.into()));
            }
            Ok(Err(why)) => return Err(DriverError::Failed(format!("the Android helper did not start: {why}"))),
            Err(_) => return Err(DriverError::Failed("the Android helper did not start in time".into())),
        }
        let ended = CancellationToken::new();
        let gone = ended.clone();
        let reader = tokio::spawn(async move {
            // What the helper prints after it started is nobody's.
            while let Ok(Some(_)) = lines.next_line().await {}
            gone.cancel();
        });
        Ok(Server { ended, reader })
    }

    /// Stops the helper the pid file names, when that process is still the
    /// helper: a pid file an emulator kept across a restart may name
    /// another process now.
    async fn stop_left(&self) -> Result<()> {
        let named = self.adb.shell(&self.serial, &format!("cat {PID_FILE} 2>/dev/null")).await?;
        if let Ok(pid) = named.trim().parse::<u32>() {
            let environment = self.adb.shell(&self.serial, &format!("cat /proc/{pid}/environ 2>/dev/null")).await?;
            if environment.split('\0').any(|variable| variable.starts_with(OURS)) {
                self.adb.shell(&self.serial, &format!("kill {pid}")).await?;
            }
        }
        self.adb.shell(&self.serial, &format!("rm -f {PID_FILE}")).await.map(drop)
    }

    /// Calls the helper's JSON-RPC `method`, starting the helper when it is
    /// not running. A helper that ended was stopped by another UI
    /// automation client, such as Appium, as it took the connection: the
    /// helper starts again only once that client let it go, which Android
    /// enforces, and until then the call fails saying so.
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        {
            let mut server = self.server.lock().await;
            if server.as_ref().is_some_and(|running| running.ended.is_cancelled()) {
                *server = None;
            }
            if server.is_none() {
                *server = Some(self.start().await?);
            }
        }
        let stream = self.adb.open(&self.serial, &format!("tcp:{PORT}")).await?;
        let failed = |error: &dyn std::fmt::Display| DriverError::Failed(format!("the Android helper's {method}: {error}"));
        let (mut sender, connection) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
            .await
            .map_err(|error| failed(&error))?;
        let connection = tokio::spawn(connection);
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
        let request = http::Request::post("/jsonrpc/0")
            .header(http::header::HOST, "localhost")
            // Without a charset the helper reads the body as ISO-8859-1.
            .header(http::header::CONTENT_TYPE, "application/json; charset=utf-8")
            .body(Full::new(Bytes::from(body)))
            .map_err(|error| failed(&error))?;
        let answered = async {
            let response = sender.send_request(request).await.map_err(|error| failed(&error))?;
            response
                .into_body()
                .collect()
                .await
                .map(|collected| collected.to_bytes())
                .map_err(|error| failed(&error))
        }
        .await;
        connection.abort();
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            result: Value,
            #[serde(default)]
            error: Option<RpcError>,
        }
        #[derive(Deserialize)]
        struct RpcError {
            #[serde(default)]
            message: String,
        }
        let answer: Answer = serde_json::from_slice(&answered?).map_err(|error| failed(&error))?;
        match answer.error {
            Some(error) => Err(failed(&error.message)),
            None => Ok(answer.result),
        }
    }
}

/// A selector of u2's that matches the focused element: the `focused`
/// field and its bit in the mask that says which fields are set.
fn focused() -> Value {
    const FOCUSED: u32 = 0x02_0000;
    json!({ "mask": FOCUSED, "childOrSibling": [], "childOrSiblingSelector": [], "focused": true })
}

impl Guest for U2 {
    fn hierarchy(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async move {
            let dumped = self.call("dumpWindowHierarchy", json!([false, 200])).await?;
            dumped
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| DriverError::Failed("the Android helper dumped no tree".into()))
        })
    }

    fn keyboard(&self) -> BoxFuture<'_, Option<String>> {
        Box::pin(async move {
            self.keyboard
                .get_or_init(|| async {
                    match self.adb.shell(&self.serial, "settings get secure default_input_method").await {
                        // `com.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME`
                        Ok(method) => method.trim().split_once('/').map(|(package, _)| package.to_owned()),
                        Err(error) => {
                            tracing::info!("the input method was not read: {error}");
                            None
                        }
                    }
                })
                .await
                .clone()
        })
    }

    fn set_text<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let set = self.call("setText", json!([focused(), text])).await?;
            if set == Value::Bool(false) {
                return Err(DriverError::Failed("no text field has the focus".into()));
            }
            Ok(())
        })
    }

    fn clipboard(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async move {
            let text = self.call("getClipboard", json!([])).await?;
            Ok(text.as_str().unwrap_or_default().to_owned())
        })
    }

    fn set_clipboard<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // The helper's first clipboard write after it starts can be
            // lost while its clipboard connection starts, so the text is
            // read back, and written once more when it is not there.
            for _ in 0..2 {
                self.call("setClipboard", json!(["demi", text])).await?;
                if self.clipboard().await? == text {
                    return Ok(());
                }
            }
            Err(DriverError::Failed("the emulator's clipboard did not take the text".into()))
        })
    }



    fn night(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async move {
            let said = self.adb.shell(&self.serial, "cmd uimode night").await?;
            Ok(said.contains("yes"))
        })
    }

    fn set_night(&self, dark: bool) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let said = self
                .adb
                .shell(&self.serial, &format!("cmd uimode night {}", if dark { "yes" } else { "no" }))
                .await?;
            match said.contains("Night mode:") {
                true => Ok(()),
                false => Err(DriverError::Failed(format!("the appearance did not change: {}", said.trim()))),
            }
        })
    }

    fn record_start(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let stream = self
                .adb
                .open(&self.serial, &format!("shell:echo $$; exec screenrecord --time-limit 0 {RECORDING} 2>&1"))
                .await?;
            let mut reader = BufReader::new(stream);
            let mut first = String::new();
            reader
                .read_line(&mut first)
                .await
                .map_err(|error| DriverError::Failed(format!("the recording did not start: {error}")))?;
            let pid = first
                .trim()
                .parse()
                .map_err(|_| DriverError::Failed(format!("the recording did not start: {}", first.trim())))?;
            let finished = tokio::spawn(async move {
                let mut rest = Vec::new();
                // What screenrecord says as it ends is the Host log's.
                if reader.read_to_end(&mut rest).await.is_ok() && !rest.is_empty() {
                    tracing::info!("screenrecord: {}", String::from_utf8_lossy(&rest).trim());
                }
            });
            *self.recording.lock().expect("the recording is intact") = Some(Recording { pid, finished });
            Ok(())
        })
    }

    fn record_stop<'a>(&'a self, file: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let recording = self
                .recording
                .lock()
                .expect("the recording is intact")
                .take()
                .ok_or_else(|| DriverError::Failed("no recording runs".into()))?;
            // An interrupted screenrecord finishes its MP4, then ends.
            self.adb.shell(&self.serial, &format!("kill -INT {}", recording.pid)).await?;
            let _finished = recording.finished.await;
            let pulled = self.adb.pull(&self.serial, RECORDING, file).await;
            self.adb.shell(&self.serial, &format!("rm -f {RECORDING}")).await?;
            pulled
        })
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let recording = self.recording.lock().expect("the recording is intact").take();
            if let Some(recording) = recording {
                recording.finished.abort();
                if let Err(error) = self.adb.shell(&self.serial, &format!("kill {}; rm -f {RECORDING}", recording.pid)).await {
                    tracing::info!("the recording did not stop: {error}");
                }
            }
            if let Some(server) = self.server.lock().await.take() {
                server.reader.abort();
                if let Err(error) = self.stop_left().await {
                    tracing::info!("the Android helper did not stop: {error}");
                }
            }
        })
    }
}

/// The tree in `xml` as the commands read it: each window's elements that
/// say or do something, in Android's display pixels; the keys of the
/// input method `keyboard` names as keyboard keys.
pub fn tree(xml: &str, keyboard: Option<&str>) -> Result<(u8, Vec<RawElement>)> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| DriverError::Failed(format!("the Android helper's tree does not read: {error}")))?;
    let root = document.root_element();
    let rotation = root.attribute("rotation").and_then(|rotation| rotation.parse::<u8>().ok()).unwrap_or(0) % 4;
    let mut elements = Vec::new();
    for window in root.children().filter(|node| node.has_tag_name("node")) {
        elements.extend(element(window, keyboard));
    }
    Ok((rotation, elements))
}

/// The text of the focused node in the helper's tree `xml`; none when no
/// node has the focus.
pub fn focused_text(xml: &str) -> Result<Option<String>> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| DriverError::Failed(format!("the Android helper's tree does not read: {error}")))?;
    Ok(document
        .descendants()
        .find(|node| node.has_tag_name("node") && node.attribute("focused") == Some("true"))
        .map(|node| node.attribute("text").unwrap_or_default().to_owned()))
}

/// `node` as the elements it holds: itself with its children when it says
/// or does something, else its children in its place.
fn element(node: roxmltree::Node<'_, '_>, keyboard: Option<&str>) -> Vec<RawElement> {
    let attribute = |name: &str| node.attribute(name).filter(|value| !value.is_empty());
    let flag = |name: &str| node.attribute(name) == Some("true");
    if node.attribute("visible-to-user") == Some("false") {
        return Vec::new();
    }
    let children: Vec<RawElement> = node
        .children()
        .filter(|child| child.has_tag_name("node"))
        .flat_map(|child| element(child, keyboard))
        .collect();
    let class = node.attribute("class").unwrap_or_default();
    let simple = class.rsplit('.').next().unwrap_or(class);
    let editable = simple.contains("EditText");
    let text = attribute("text");
    let description = attribute("content-desc");
    let identifier = attribute("resource-id").map(|id| id.split_once(":id/").map_or(id, |(_, name)| name).to_owned());
    let acts = flag("clickable") || flag("long-clickable") || flag("scrollable") || flag("checkable") || editable;
    if text.is_none() && description.is_none() && !acts {
        return children;
    }
    let (label, value) = if editable {
        (description.or(attribute("hint")).map(str::to_owned), text.map(str::to_owned))
    } else {
        (description.or(text).map(str::to_owned), None)
    };
    let in_keyboard = keyboard.is_some_and(|keyboard| node.attribute("package") == Some(keyboard));
    vec![RawElement {
        role: role(simple, flag("clickable"), flag("scrollable")).to_owned(),
        label,
        value,
        identifier,
        frame: bounds(node.attribute("bounds").unwrap_or_default()),
        enabled: Some(!(node.attribute("enabled") == Some("false"))),
        selected: Some(flag("selected") || flag("checked")),
        focused: Some(flag("focused")),
        secure: flag("password"),
        keyboard_key: in_keyboard,
        children,
    }]
}

/// The role of a view of class `simple`, as the tree names roles.
fn role(simple: &str, clickable: bool, scrollable: bool) -> &'static str {
    match simple {
        name if name.contains("EditText") => "text field",
        name if name.contains("RadioButton") => "radio button",
        name if name.contains("CheckBox") => "check box",
        name if name.contains("Switch") || name.contains("ToggleButton") => "switch",
        name if name.contains("Button") => "button",
        name if name.contains("SeekBar") => "slider",
        name if name.contains("ProgressBar") => "progress indicator",
        name if name.contains("WebView") => "web view",
        _ if clickable => "button",
        _ if scrollable => "scroll view",
        name if name.contains("Image") => "image",
        name if name.contains("TextView") => "text",
        _ => "group",
    }
}

/// `[left,top][right,bottom]` as a rectangle.
fn bounds(text: &str) -> Rect {
    let numbers: Vec<f64> = text
        .split(|character: char| !(character.is_ascii_digit() || character == '-'))
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect();
    match numbers.as_slice() {
        [left, top, right, bottom] => Rect {
            x: *left,
            y: *top,
            width: (right - left).max(0.0),
            height: (bottom - top).max(0.0),
        },
        _ => Rect::default(),
    }
}
