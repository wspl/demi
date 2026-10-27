//! What Demi does inside an emulator over adb
//! (`crates/plugin-mobile/command/src/android/mod.rs`):
//!
//! - the UI tree with its display rotation, a field's text and the clipboard,
//!   through openatx's `u2.jar`, which `demi-mobile` carries and runs with
//!   `app_process` as a shell process, not an installed app;
//! - appearance, the keyboard's tutorials, and keys where the emulator has
//!   no hardware keyboard to take its own, through the shell.

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use futures_util::{StreamExt as _, future::BoxFuture, stream::BoxStream};
use http_body_util::{BodyExt as _, Full};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::adb::Adb;
use crate::driver::{DriverError, RawElement, Rect, Result};

/// The largest APK copied from the device to read its files: SystemUI's
/// is about 40 MB, and the copy is written to this computer's disk.
const APK_LIMIT: u64 = 256 * 1024 * 1024;
/// The largest file read from an APK, uncompressed: a corner's drawable is
/// under a kilobyte, an app's manifest a few hundred, and a compressed file
/// can declare any size.
const APK_FILE_LIMIT: u64 = 1024 * 1024;

/// A resource as the device resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// A value as Android prints it, such as `132.0px`, or a file's path in
    /// the APK that supplies it, such as `res/drawable/rounded_corner_top.xml`.
    pub value: String,
    /// The device's path of the APK the value comes from, the overlay that
    /// replaces it or the package's own; none where Android does not say.
    pub apk: Option<String>,
}

/// The port the helper listens on inside the emulator; another than the
/// 9008 uiautomator2's own clients use, so the two do not collide.
const PORT: u16 = 9018;
/// Where the helper's process id is kept, so the next program stops the
/// helper this one left.
const PID_FILE: &str = "/data/local/tmp/demi-u2.pid";
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
/// Appium, holds the UI automation connection (`crates/plugin-mobile/command/src/android/mod.rs`).
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
    /// Whether the display is on now, then each time it turns on or off,
    /// as Android announces it, until the stream is dropped or the
    /// emulator ends.
    fn display(&self) -> BoxFuture<'_, Result<BoxStream<'static, bool>>>;
    /// Whether a touchscreen takes touches on the display Android shows
    /// first, a display of `pixels`, portrait: Android drops what a
    /// touchscreen sends while its display is not the one shown, and a
    /// foldable that changed the display it shows takes the other's
    /// touchscreen up some time after.
    fn touchable(&self, pixels: (f64, f64)) -> BoxFuture<'_, Result<bool>>;
    /// Presses the wake key, which turns the screen on and leaves one
    /// that is on as it is.
    fn wake(&self) -> BoxFuture<'_, Result<()>>;
    /// Presses the power key, as the side button: a display that is on
    /// turns off, one that is off turns on.
    fn power(&self) -> BoxFuture<'_, Result<()>>;
    /// Whether the emulator has a hardware keyboard, which the keys the
    /// emulator sends need: without one they reach nothing.
    fn hardware_keyboard(&self) -> BoxFuture<'_, Result<bool>>;
    /// Presses each of `presses` in order through Android's own input
    /// injection, which needs no hardware keyboard: the keys of each, by
    /// their `KEYCODE_` names, pressed together.
    fn inject_keys<'a>(&'a self, presses: &'a [Vec<&'static str>]) -> BoxFuture<'a, Result<()>>;
    /// Turns off the keyboard's first-run tutorials that cover the screen,
    /// as a person does with a tap the first time: Gboard's stylus
    /// handwriting tutorial.
    fn skip_keyboard_tutorials(&self) -> BoxFuture<'_, Result<()>>;
    /// The text on the clipboard, empty when it holds none.
    fn clipboard(&self) -> BoxFuture<'_, Result<String>>;
    fn set_clipboard<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>>;

    /// Whether the dark appearance is on.
    fn night(&self) -> BoxFuture<'_, Result<bool>>;
    fn set_night(&self, dark: bool) -> BoxFuture<'_, Result<()>>;
    /// A resource of `package` as Android resolves it, its overlays
    /// included (`cmd overlay lookup`), with the APK it resolves to.
    fn resource<'a>(&'a self, package: &'a str, name: &'a str) -> BoxFuture<'a, Result<Resolved>>;
    /// The files `paths` of the device's APK `apk`, none for one it does
    /// not hold; the APK is copied from the device once for all of them.
    /// An APK, or a file in it, larger than its limit does not read.
    fn apk_files<'a>(&'a self, apk: &'a str, paths: &'a [String]) -> BoxFuture<'a, Result<Vec<Option<Vec<u8>>>>>;
    /// Installs the APK `file` of this computer as `adb install -r -t`
    /// does: streamed to the package manager, replacing an installed
    /// version and keeping its data, a build marked for tests too.
    fn install<'a>(&'a self, file: &'a std::path::Path) -> BoxFuture<'a, Result<()>>;
    /// Starts `package`'s launcher activity as the launcher does, which
    /// brings a running app's task in front, after stopping the app when
    /// `restart`; answers once its first screen shows, whether its process
    /// started anew.
    fn launch<'a>(&'a self, package: &'a str, restart: bool) -> BoxFuture<'a, Result<bool>>;
    /// The user id of the installed `package`, which all its processes run
    /// as.
    fn uid<'a>(&'a self, package: &'a str) -> BoxFuture<'a, Result<u32>>;
    /// The log's lines from `since`, in seconds since the epoch, as
    /// `logcat -v epoch,uid,usec` prints them, every level, until the
    /// stream is dropped or the emulator ends.
    fn log(&self, since: f64) -> BoxFuture<'_, Result<BoxStream<'static, String>>>;
    /// Stops what Demi runs inside the emulator.
    fn close(&self) -> BoxFuture<'_, ()>;
}

/// `package` when it is a package name, which goes into a shell command as
/// it is.
fn package_name(package: &str) -> Result<&str> {
    let named = !package.is_empty() && package.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
    if named {
        Ok(package)
    } else {
        Err(DriverError::Failed(format!("{package:?} is not an Android package name, such as com.example.app")))
    }
}

/// The helper's process inside the emulator, while its shell connection
/// stays open.
struct Server {
    /// Cancelled once the process ended.
    ended: CancellationToken,
    /// Reads what the helper prints until it ends; aborted as Demi lets go.
    reader: tokio::task::JoinHandle<()>,
}

/// The emulator `serial`, reached through the adb server.
pub struct U2 {
    adb: Adb,
    serial: String,
    server: tokio::sync::Mutex<Option<Server>>,
    /// The input method's package, read once.
    keyboard: tokio::sync::OnceCell<Option<String>>,
}

impl U2 {
    pub fn new(adb: Adb, serial: String) -> Arc<Self> {
        Arc::new(Self {
            adb,
            serial,
            server: tokio::sync::Mutex::default(),
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
                    tracing::info!(event = "android_helper.stop_failed", case = "refused", %error, "the Android helper did not stop");
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
                            tracing::info!(event = "android_input_method.read_failed", %error, "the input method was not read");
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

    fn hardware_keyboard(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async move { Ok(hardware_keyboard(&self.adb.shell(&self.serial, "dumpsys input").await?)) })
    }

    fn inject_keys<'a>(&'a self, presses: &'a [Vec<&'static str>]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // One shell for all of them: each `input` waits until Android
            // dispatched its keys, so they arrive in order.
            let mut commands = Vec::new();
            let mut alone: Vec<&str> = Vec::new();
            for press in presses {
                match press.as_slice() {
                    [key] => alone.push(key),
                    keys => {
                        if !alone.is_empty() {
                            commands.push(format!("input keyevent {}", alone.join(" ")));
                            alone.clear();
                        }
                        commands.push(format!("input keycombination {}", keys.join(" ")));
                    }
                }
            }
            if !alone.is_empty() {
                commands.push(format!("input keyevent {}", alone.join(" ")));
            }
            if commands.is_empty() {
                return Ok(());
            }
            // `input` says nothing unless it failed.
            let said = self.adb.shell(&self.serial, &commands.join(" && ")).await?;
            match said.trim() {
                "" => Ok(()),
                said => Err(DriverError::Failed(format!("Android did not take the keys: {said}"))),
            }
        })
    }

    fn skip_keyboard_tutorials(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.adb
                .shell(&self.serial, "settings put secure stylus_handwriting_enabled 0")
                .await
                .map(drop)
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



    fn resource<'a>(&'a self, package: &'a str, name: &'a str) -> BoxFuture<'a, Result<Resolved>> {
        Box::pin(async move {
            let said = self
                .adb
                .shell(&self.serial, &format!("cmd overlay lookup --verbose {package} {name}"))
                .await?;
            resolved(&said).ok_or_else(|| DriverError::Failed(format!("{name} does not resolve: {}", said.trim())))
        })
    }

    fn apk_files<'a>(&'a self, apk: &'a str, paths: &'a [String]) -> BoxFuture<'a, Result<Vec<Option<Vec<u8>>>>> {
        Box::pin(async move {
            let copy = tempfile::NamedTempFile::new()
                .map_err(|error| DriverError::Failed(format!("no temporary file for an APK: {error}")))?;
            self.adb.pull_at_most(&self.serial, apk, copy.path(), APK_LIMIT).await?;
            let paths = paths.to_vec();
            tokio::task::spawn_blocking(move || read_apk(copy.as_file(), &paths))
                .await
                .map_err(|error| DriverError::Failed(format!("reading an APK stopped: {error}")))?
        })
    }

    fn touchable(&self, pixels: (f64, f64)) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async move {
            let viewports = self.adb.shell(&self.serial, "dumpsys input | grep 'Viewport INTERNAL: displayId=0,'").await?;
            Ok(takes_touches(&viewports, pixels))
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

    fn display(&self) -> BoxFuture<'_, Result<BoxStream<'static, bool>>> {
        Box::pin(async move {
            // Android logs each turn of the display in its event log as
            // `screen_toggled`, 1 or 0. The log is followed from now first,
            // then the state is read, so no turn falls between them; a turn
            // in both is told twice, which changes nothing.
            let followed = self
                .adb
                .open(
                    &self.serial,
                    "shell:exec logcat -b events -v raw -s screen_toggled -T \"$(date '+%m-%d %H:%M:%S.000')\"",
                )
                .await?;
            let power = self.adb.shell(&self.serial, "dumpsys power").await?;
            let now = awake(&power)?;
            let turns = futures_util::stream::unfold(BufReader::new(followed).lines(), |mut lines| async move {
                loop {
                    // The log ends with the emulator, or with the stream.
                    let line = lines.next_line().await.ok()??;
                    match line.trim() {
                        "1" => return Some((true, lines)),
                        "0" => return Some((false, lines)),
                        // The log's own lines, such as its buffer's start.
                        _ => {}
                    }
                }
            });
            Ok(futures_util::stream::once(std::future::ready(now)).chain(turns).boxed())
        })
    }

    fn wake(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            // KEYCODE_WAKEUP, 224: the emulator's own keys have no wake key.
            self.adb.shell(&self.serial, "input keyevent KEYCODE_WAKEUP").await.map(drop)
        })
    }

    fn power(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            // KEYCODE_POWER, 26: the emulator's own Power key, sent over its
            // gRPC interface, reaches an Android 16 guest as nothing.
            self.adb.shell(&self.serial, "input keyevent KEYCODE_POWER").await.map(drop)
        })
    }

    fn install<'a>(&'a self, file: &'a std::path::Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            use tokio::io::AsyncReadExt as _;
            let unread = |error: std::io::Error| DriverError::Failed(format!("{} does not read: {error}", file.display()));
            let mut apk = tokio::fs::File::open(file).await.map_err(unread)?;
            let size = apk.metadata().await.map_err(unread)?.len();
            // The package manager reads the APK's bytes from its standard
            // input, as many as -S says.
            let mut installer = self
                .adb
                .open(&self.serial, &format!("exec:cmd package install -S {size} -r -t"))
                .await?;
            tokio::io::copy(&mut apk, &mut installer)
                .await
                .map_err(|error| DriverError::Failed(format!("the APK did not reach the emulator: {error}")))?;
            let mut said = String::new();
            installer
                .read_to_string(&mut said)
                .await
                .map_err(|error| DriverError::Failed(format!("the package manager did not answer: {error}")))?;
            match said.lines().map(str::trim).find(|line| line.starts_with("Success") || line.starts_with("Failure")) {
                Some(line) if line.starts_with("Success") => Ok(()),
                Some(failure) => Err(DriverError::Failed(format!("the package manager refused the APK: {failure}"))),
                None => Err(DriverError::Failed(format!("the package manager answered {:?}", said.trim()))),
            }
        })
    }

    fn launch<'a>(&'a self, package: &'a str, restart: bool) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            let package = package_name(package)?;
            let resolved = self
                .adb
                .shell(
                    &self.serial,
                    &format!("cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.LAUNCHER {package}"),
                )
                .await?;
            let component = resolved
                .lines()
                .map(str::trim)
                .rfind(|line| line.starts_with(&format!("{package}/")))
                .filter(|component| !component.contains('\''))
                .ok_or_else(|| {
                    DriverError::Failed(format!("{package} is not installed, or has no activity the launcher starts"))
                })?;
            // The launcher's own flags, NEW_TASK and RESET_TASK_IF_NEEDED,
            // bring a running app's task in front as it was left.
            let started = self
                .adb
                .shell(
                    &self.serial,
                    &format!(
                        "am start -W {}-a android.intent.action.MAIN -c android.intent.category.LAUNCHER -f 0x10200000 -n '{component}'",
                        if restart { "-S " } else { "" }
                    ),
                )
                .await?;
            if let Some(error) = started.lines().map(str::trim).find(|line| line.starts_with("Error")) {
                return Err(DriverError::Failed(format!("{package} did not start: {error}")));
            }
            Ok(started.lines().any(|line| line.trim() == "LaunchState: COLD"))
        })
    }

    fn uid<'a>(&'a self, package: &'a str) -> BoxFuture<'a, Result<u32>> {
        Box::pin(async move {
            let package = package_name(package)?;
            let listed = self.adb.shell(&self.serial, &format!("cmd package list packages -U {package}")).await?;
            listed
                .lines()
                .filter_map(|line| line.trim().strip_prefix("package:"))
                .filter_map(|line| line.split_once(" uid:"))
                .find(|(name, _)| *name == package)
                .and_then(|(_, uid)| uid.trim().split(',').next()?.parse().ok())
                .ok_or_else(|| DriverError::Failed(format!("{package} is not installed")))
        })
    }

    fn log(&self, since: f64) -> BoxFuture<'_, Result<BoxStream<'static, String>>> {
        Box::pin(async move {
            let followed = self
                .adb
                .open(&self.serial, &format!("shell:exec logcat -v epoch,uid,usec -T {since:.6}"))
                .await?;
            // A line of a message that is not UTF-8 reads with its bytes
            // replaced, never ends the log.
            let lines = futures_util::stream::unfold(BufReader::new(followed), |mut reader| async move {
                let mut line = Vec::new();
                match reader.read_until(b'\n', &mut line).await {
                    Ok(0) | Err(_) => None,
                    Ok(_) => Some((String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_owned(), reader)),
                }
            });
            Ok(lines.boxed())
        })
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Some(server) = self.server.lock().await.take() {
                server.reader.abort();
                if let Err(error) = self.stop_left().await {
                    tracing::info!(event = "android_helper.stop_failed", case = "close", %error, "the Android helper did not stop");
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

/// Whether one of the input viewports Android lists in `viewports`, as
/// `dumpsys input` writes them, is the first display's, its logical frame of
/// `pixels`, portrait or turned, and active: Android 12 and later say whether
/// one is; an earlier Android lists the display's only while it is.
fn takes_touches(viewports: &str, pixels: (f64, f64)) -> bool {
    let size = |line: &str| {
        let from = line.find("logicalFrame=[")? + "logicalFrame=[".len();
        let frame: Vec<f64> = line[from..].split(']').next()?.split(',').filter_map(|part| part.trim().parse().ok()).collect();
        match frame.as_slice() {
            [left, top, right, bottom] => Some((right - left, bottom - top)),
            _ => None,
        }
    };
    viewports.lines().filter(|line| line.contains("displayId=0,")).any(|line| {
        let active = !line.contains("isActive=[0]");
        let fits = size(line).is_some_and(|(width, height)| {
            ((width - pixels.0).abs() < 1.0 && (height - pixels.1).abs() < 1.0)
                || ((width - pixels.1).abs() < 1.0 && (height - pixels.0).abs() < 1.0)
        });
        active && fits
    })
}

/// The text of the focused node in the helper's tree `xml`; none when no
/// node has the focus.
pub fn focused_text(xml: &str) -> Result<Option<String>> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| DriverError::Failed(format!("the Android helper's tree does not read: {error}")))?;
    Ok(document
        .descendants()
        .find(|node| node.has_tag_name("node") && node.attribute("focused") == Some("true"))
        .map(|node| text(node).unwrap_or_default().to_owned()))
}

/// The text `node` holds; none when its text is its hint, which an empty
/// field says as its text (`RawElement::hint`).
fn text<'a>(node: roxmltree::Node<'a, '_>) -> Option<&'a str> {
    node.attribute("text")
        .filter(|text| !text.is_empty() && node.attribute("hint") != Some(text))
}

/// Whether `dumpsys input` says the emulator has a hardware keyboard: an
/// alphabetic keyboard (KeyboardType 2) other than the `Virtual` one Android
/// always has, which only injected keys reach.
pub fn hardware_keyboard(dumped: &str) -> bool {
    let mut device = "";
    for line in dumped.lines().map(str::trim) {
        if let Some((number, name)) = line.strip_prefix("Device ").and_then(|rest| rest.split_once(": "))
            && number.parse::<i64>().is_ok()
        {
            device = name.trim();
        } else if let Some(kind) = line.strip_prefix("KeyboardType: ")
            && (kind == "2" || kind.eq_ignore_ascii_case("alphabetic"))
            && device != "Virtual"
        {
            return true;
        }
    }
    false
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
    let text = text(node);
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
        hint: editable.then(|| attribute("hint").map(str::to_owned)).flatten(),
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

/// Whether `dumpsys power` says the display is on: awake or dreaming,
/// whose screen saver shows; asleep and dozing are off.
fn awake(power: &str) -> Result<bool> {
    let wakefulness = power
        .lines()
        .find_map(|line| line.trim().strip_prefix("mWakefulness="))
        .ok_or_else(|| DriverError::Failed("the display's power state does not read".into()))?;
    Ok(matches!(wakefulness.trim(), "Awake" | "Dreaming"))
}

/// What `cmd overlay lookup --verbose` prints of a resource: the APKs it
/// was looked up in, `Found initial:` the package's and `Overlaid:` each
/// overlay's that replaces it, each a path and an entry number, and its
/// value on the last line; an unknown resource prints an error, or nothing.
pub fn resolved(said: &str) -> Option<Resolved> {
    let value = said.lines().map(str::trim).rfind(|line| !line.is_empty())?;
    if value.starts_with("Error") || value.contains("Exception") {
        return None;
    }
    let apk = said
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix("Overlaid:").or_else(|| line.strip_prefix("Found initial:"))
        })
        .last()
        .map(|apk| apk.trim().rsplit_once(" #").map_or(apk.trim(), |(path, _)| path).to_owned());
    Some(Resolved { value: value.to_owned(), apk })
}

/// The files `paths` of the APK `apk`, none for one it does not hold,
/// each read where the archive's central directory places it and no more
/// than [`APK_FILE_LIMIT`] of it.
pub fn read_apk(apk: &std::fs::File, paths: &[String]) -> Result<Vec<Option<Vec<u8>>>> {
    use std::io::Read as _;
    let unreadable = |error: &dyn std::fmt::Display| DriverError::Failed(format!("an APK does not read: {error}"));
    let mut archive = zip::ZipArchive::new(apk).map_err(|error| unreadable(&error))?;
    paths
        .iter()
        .map(|path| {
            let file = match archive.by_name(path) {
                Ok(file) => file,
                Err(zip::result::ZipError::FileNotFound) => return Ok(None),
                Err(error) => return Err(unreadable(&error)),
            };
            // The size the directory states is the device's word; the read
            // stops past the limit whatever it says.
            let mut bytes = Vec::new();
            file.take(APK_FILE_LIMIT + 1).read_to_end(&mut bytes).map_err(|error| unreadable(&error))?;
            if bytes.len() as u64 > APK_FILE_LIMIT {
                return Err(unreadable(&format!("{path} is larger than {APK_FILE_LIMIT} bytes")));
            }
            Ok(Some(bytes))
        })
        .collect()
}
