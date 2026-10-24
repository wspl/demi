//! What the iOS driver asks of `xcrun simctl` itself: erasing a simulator,
//! installing and launching apps, and following its unified log, as Xcode
//! and Simulator do. The helper's FBSimulatorControl launches an app
//! without its console and follows no log, so these are simctl's.
//!
//! Every process this starts ends as the future or the task that owns it
//! is dropped. A `log stream` is asked to end, since `simctl spawn` hands
//! the request on to it inside the simulator, where one killed with its
//! simctl stays until it next writes. The console of an app Demi launched
//! is killed, which leaves the app running.

use std::{collections::HashMap, path::Path, process::Stdio, sync::Arc, time::Duration};

use demi_command_package_mobile_protocol::mobile::{LogEntry, LogLevel};
use serde::Deserialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, Command},
};
use tokio_util::sync::CancellationToken;

use crate::{
    driver::{DriverError, InstalledApp, Launched, Result},
    log::Log,
};

/// How long the log waits to be followed again once `log stream` ended.
const LOG_AGAIN: Duration = Duration::from_secs(1);
/// How long a launch waits for simctl to say the app's process: the app's
/// launch itself, which a first launch after an install makes slow.
const LAUNCHING: Duration = Duration::from_secs(60);
/// How often a launch looks for the app's new process.
const LAUNCH_POLL: Duration = Duration::from_millis(100);

fn simctl(args: &[&str]) -> Command {
    let mut command = Command::new("xcrun");
    command.arg("simctl").args(args).stdin(Stdio::null()).kill_on_drop(true);
    command
}

/// Runs `simctl <args>` and answers what it printed; a failure says the
/// line of simctl's that tells why.
async fn run(args: &[&str]) -> Result<String> {
    let output = simctl(args)
        .output()
        .await
        .map_err(|error| DriverError::Failed(format!("simctl did not start: {error}")))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        // simctl says what failed on its second line, after its generic one.
        let reason = said
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("An error was encountered"))
            .unwrap_or("simctl failed");
        return Err(DriverError::Failed(reason.to_owned()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Erases the shut-down simulator `udid`, as Erase All Content and
/// Settings does.
pub async fn erase(udid: &str) -> Result<()> {
    run(&["erase", udid]).await.map(drop)
}

/// A value of the app bundle `app`'s Info.plist.
async fn bundle_value(app: &Path, key: &str) -> Option<String> {
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(app.join("Info.plist"))
        .stdin(Stdio::null())
        .output()
        .await
        .ok()?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !value.is_empty()).then_some(value)
}

/// Installs the app bundle `app` on `udid`, replacing an installed version
/// and keeping its data.
pub async fn install(udid: &str, app: &Path) -> Result<InstalledApp> {
    if !app.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("app")) || !app.is_dir() {
        return Err(DriverError::Failed(format!(
            "an iOS simulator installs an .app, the folder a simulator build makes, and {} is not one",
            app.display()
        )));
    }
    let id = bundle_value(app, "CFBundleIdentifier")
        .await
        .ok_or_else(|| DriverError::Failed(format!("{} has no Info.plist that names its bundle identifier", app.display())))?;
    let path = app.to_string_lossy();
    run(&["install", udid, &path]).await?;
    Ok(InstalledApp {
        app: id,
        version: bundle_value(app, "CFBundleShortVersionString").await,
    })
}

/// What simctl says of an installed app.
pub struct AppInfo {
    /// Its executable, which its process is named by.
    pub executable: String,
    /// Its bundle's folder.
    pub path: String,
    /// The name its icon shows, which its window is named by.
    pub name: Option<String>,
}

/// The installed app `id` of `udid`, from `simctl appinfo`'s property list
/// in the old text form, a `key = value;` line per value.
pub async fn app_info(udid: &str, id: &str) -> Result<AppInfo> {
    let said = run(&["appinfo", udid, id])
        .await
        .map_err(|error| DriverError::Failed(format!("{id} is not installed: {error}")))?;
    let value = |key: &str| {
        said.lines().find_map(|line| {
            let value = line.trim().strip_prefix(key)?.trim_start().strip_prefix('=')?.trim().strip_suffix(';')?;
            Some(value.strip_prefix('"').and_then(|value| value.strip_suffix('"')).unwrap_or(value).replace("\\\"", "\""))
        })
    };
    match (value("CFBundleExecutable"), value("Path")) {
        (Some(executable), Some(path)) => Ok(AppInfo {
            executable,
            path,
            name: value("CFBundleDisplayName").or_else(|| value("CFBundleName")),
        }),
        _ => Err(DriverError::Failed(format!("simctl does not say where {id} is installed"))),
    }
}

/// The process of the app `id` on `udid` while it runs: launchd lists
/// each app it started as `UIKitApplication:<id>[…]`, with its process.
async fn process(udid: &str, id: &str) -> Result<Option<u32>> {
    let listed = run(&["spawn", udid, "launchctl", "list"]).await?;
    let label = format!("UIKitApplication:{id}[");
    Ok(listed.lines().find_map(|line| {
        let mut columns = line.split_whitespace();
        let pid = columns.next()?.parse().ok()?;
        columns.nth(1).filter(|name| name.starts_with(&label)).map(|_| pid)
    }))
}

/// One entry of the unified log, as `log stream --style ndjson` prints it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    event_type: String,
    #[serde(default)]
    message_type: Option<String>,
    timestamp: String,
    #[serde(default)]
    process_image_path: Option<String>,
    #[serde(rename = "processID")]
    process_id: u32,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    event_message: String,
}

/// The entry of a line of `log stream --style ndjson`; none for its other
/// lines, such as the filter it says it uses, and for events other than
/// messages, such as an activity's start.
fn unified(line: &str) -> Option<LogEntry> {
    let event: Event = serde_json::from_str(line).ok()?;
    if event.event_type != "logEvent" {
        return None;
    }
    let level = match event.message_type.as_deref() {
        Some("Debug") => LogLevel::Debug,
        Some("Info") => LogLevel::Info,
        Some("Error") => LogLevel::Error,
        Some("Fault") => LogLevel::Fault,
        _ => LogLevel::Notice,
    };
    let timestamp = jiff::fmt::strtime::parse("%Y-%m-%d %H:%M:%S%.f%z", &event.timestamp)
        .and_then(|parsed| parsed.to_timestamp())
        .map_or_else(|_| now(), |timestamp| timestamp.as_millisecond() as f64);
    let mut entry = crate::log::entry(timestamp, level, event.process_id, event.event_message);
    entry.process = event
        .process_image_path
        .as_deref()
        .and_then(|path| path.rsplit('/').next())
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    entry.tag = event.category.filter(|category| !category.is_empty());
    Some(entry)
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64() * 1000.0
}

/// The longest line kept whole: an app may print without end and never a
/// newline, and Android's log cuts an entry at about as much.
const LINE_BYTES: usize = 4096;

/// The next line `reader` reads, at most [`LINE_BYTES`] of it, a longer one
/// going on in the next; its bytes that are not UTF-8 replaced and a
/// terminal's carriage return taken off. None once it ends.
async fn next_line(reader: &mut BufReader<impl AsyncRead + Unpin>) -> Option<String> {
    let text = |line: &[u8]| String::from_utf8_lossy(line).trim_end_matches('\r').to_owned();
    let mut line = Vec::new();
    loop {
        let available = match reader.fill_buf().await {
            Ok(available) if !available.is_empty() => available,
            _ => return (!line.is_empty()).then(|| text(&line)),
        };
        let room = LINE_BYTES - line.len();
        if let Some(end) = available.iter().take(room).position(|byte| *byte == b'\n') {
            line.extend_from_slice(&available[..end]);
            reader.consume(end + 1);
            return Some(text(&line));
        }
        let taken = available.len().min(room);
        line.extend_from_slice(&available[..taken]);
        reader.consume(taken);
        if line.len() == LINE_BYTES {
            return Some(text(&line));
        }
    }
}

/// Reads the lines `output` prints into `each`, until it ends.
async fn lines(output: impl AsyncRead + Unpin, mut each: impl FnMut(String)) {
    let mut reader = BufReader::new(output);
    while let Some(line) = next_line(&mut reader).await {
        each(line);
    }
}

/// A running `log stream`, asked to end as it drops.
struct Stream(Child);

impl Drop for Stream {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0.id().and_then(|pid| libc::pid_t::try_from(pid).ok()) {
            // SAFETY: a signal to the child this owns, which has not been
            // waited for, so its id is still its own.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

/// Starts `log stream` on `udid` with `args`, its JSON lines on its
/// standard output.
fn stream(udid: &str, args: &[&str]) -> Result<(Stream, tokio::process::ChildStdout)> {
    let mut command = simctl(&["spawn", udid, "log", "stream", "--style", "ndjson"]);
    command.args(args).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(false);
    let mut child =
        command.spawn().map_err(|error| DriverError::Failed(format!("the simulator's log did not start: {error}")))?;
    let output = child.stdout.take().expect("its output is piped");
    Ok((Stream(child), output))
}

/// How long `log stream` takes to say it follows the log.
const STREAM_READY: Duration = Duration::from_secs(5);

/// Starts `log stream` on `udid` with `args` and waits until it follows the
/// log, which it says on a line of its own before its entries.
async fn ready(udid: &str, args: &[&str]) -> Result<(Stream, BufReader<tokio::process::ChildStdout>)> {
    let (child, output) = stream(udid, args)?;
    let mut output = BufReader::new(output);
    tokio::time::timeout(STREAM_READY, async {
        while let Some(line) = next_line(&mut output).await {
            if line.starts_with("Filtering the log data") {
                return Ok(());
            }
        }
        Err(DriverError::Failed("the simulator's log ended as it started".into()))
    })
    .await
    .map_err(|_| DriverError::Failed(format!("the simulator's log did not start in {} s", STREAM_READY.as_secs())))??;
    Ok((child, output))
}

/// Follows `udid`'s unified log at its default level into `log` until
/// `stop`, starting it again when it ends while the simulator runs.
pub async fn follow(udid: String, log: Arc<Log>, stop: CancellationToken) {
    loop {
        match stream(&udid, &["--level", "default"]) {
            Ok((_child, output)) => {
                tokio::select! {
                    () = stop.cancelled() => return,
                    () = lines(output, |line| if let Some(entry) = unified(&line) { log.push(entry) }) => {}
                }
            }
            Err(error) => tracing::info!(event = "ios_log.follow_failed", %error, "the simulator's log was not followed"),
        }
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(LOG_AGAIN) => {}
        }
    }
}

/// The apps of a simulator Demi launched, each followed until it ends, is
/// launched again or its device closes, across the device's connections.
#[derive(Default)]
pub struct Launches {
    apps: std::sync::Mutex<HashMap<String, CancellationToken>>,
}

impl Launches {
    /// Whether Demi follows `id`'s console, as it does from its launch
    /// until it ends.
    pub fn following(&self, id: &str) -> bool {
        self.apps.lock().expect("the launches are intact").get(id).is_some_and(|ended| !ended.is_cancelled())
    }

    /// Launches `id` on `udid`, as a tap on its icon does, or anew when
    /// `restart` or when it does not run: then with its console, which
    /// goes into `log` with its own debug and info messages until it ends
    /// or the device closes. Answers what it did and the app's process.
    pub async fn launch(&self, udid: &str, id: &str, restart: bool, log: &Arc<Log>) -> Result<(Launched, AppInfo)> {
        let info = app_info(udid, id).await?;
        let before = process(udid, id).await?;
        if !restart && before.is_some() {
            run(&["launch", udid, id]).await?;
            let launched = Launched {
                started: false,
                console: self.following(id),
                shown: true,
            };
            return Ok((launched, info));
        }
        // The app's earlier launch ends with it.
        if let Some(earlier) = self.apps.lock().expect("the launches are intact").remove(id) {
            earlier.cancel();
        }
        let ended = log.closed().child_token();
        // Its own debug and info messages, which the default level leaves
        // out: those of its bundle's code, not of the system's frameworks
        // it loads, which log far more. Followed before it starts, so its
        // first ones are there, and again while it runs when the stream
        // ends.
        let quoted = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
        let predicate = format!(
            "process == \"{}\" AND senderImagePath BEGINSWITH \"{}/\" AND (messageType == debug OR messageType == info)",
            quoted(&info.executable),
            quoted(&info.path)
        );
        let args = ["--level".to_owned(), "debug".to_owned(), "--predicate".to_owned(), predicate];
        let first = ready(udid, &args.iter().map(String::as_str).collect::<Vec<_>>()).await;
        tokio::spawn(follow_messages(udid.to_owned(), args, first, log.clone(), ended.clone()));

        let mut child = simctl(&["launch", "--console-pty", "--terminate-running-process", udid, id])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| DriverError::Failed(format!("simctl did not start: {error}")))?;
        let output = child.stdout.take().expect("its output is piped");
        let mut stderr = child.stderr.take().expect("its errors are piped");
        // What the app prints goes to the log as it comes, from the start,
        // so the pipe never fills, under the app's process once known.
        let pid = Arc::new(std::sync::atomic::AtomicU32::new(0));
        tokio::spawn(follow_console(child, output, info.executable.clone(), id.to_owned(), pid.clone(), log.clone(), ended.clone()));
        // simctl says the app's process only as it ends, its own output
        // buffered into a pipe, so the process is launchd's new one.
        let deadline = tokio::time::Instant::now() + LAUNCHING;
        let found = loop {
            if ended.is_cancelled() {
                let mut said = String::new();
                use tokio::io::AsyncReadExt as _;
                let _read = tokio::time::timeout(Duration::from_secs(1), stderr.read_to_string(&mut said)).await;
                let reason = said
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty() && !line.starts_with("An error was encountered"))
                    .unwrap_or("it ended as it started; logs holds what it printed");
                break Err(DriverError::Failed(format!("{id} did not launch: {reason}")));
            }
            if let Some(found) = process(udid, id).await?.filter(|found| Some(*found) != before) {
                break Ok(found);
            }
            if tokio::time::Instant::now() >= deadline {
                break Err(DriverError::Failed(format!("{id} did not start in {} s", LAUNCHING.as_secs())));
            }
            tokio::time::sleep(LAUNCH_POLL).await;
        };
        match found {
            Ok(found) => pid.store(found, std::sync::atomic::Ordering::Relaxed),
            Err(error) => {
                ended.cancel();
                return Err(error);
            }
        }
        self.apps.lock().expect("the launches are intact").insert(id.to_owned(), ended);
        let launched = Launched {
            started: true,
            console: true,
            shown: false,
        };
        Ok((launched, info))
    }
}

/// Follows the app's own messages with `log stream <args>`, from `first`,
/// until `ended`, starting the stream again when it ends while the app
/// runs.
async fn follow_messages(
    udid: String,
    args: [String; 4],
    first: Result<(Stream, BufReader<tokio::process::ChildStdout>)>,
    log: Arc<Log>,
    ended: CancellationToken,
) {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut next = Some(first);
    loop {
        let started = match next.take() {
            Some(started) => started,
            None => ready(&udid, &args).await,
        };
        match started {
            Ok((_stream, mut output)) => {
                let read = async {
                    while let Some(line) = next_line(&mut output).await {
                        if let Some(entry) = unified(&line) {
                            log.push(entry);
                        }
                    }
                };
                tokio::select! {
                    () = ended.cancelled() => return,
                    () = read => {}
                }
            }
            Err(error) => tracing::info!(event = "ios_log.follow_failed", %error, "an app's own messages were not followed"),
        }
        tokio::select! {
            () = ended.cancelled() => return,
            () = tokio::time::sleep(LOG_AGAIN) => {}
        }
    }
}

/// Puts what the app prints into `log` until its console ends, which ends
/// `ended` too, or `ended` does, which kills the console and leaves the app
/// running.
async fn follow_console(
    child: Child,
    output: tokio::process::ChildStdout,
    executable: String,
    id: String,
    pid: Arc<std::sync::atomic::AtomicU32>,
    log: Arc<Log>,
    ended: CancellationToken,
) {
    let mut output = BufReader::new(output);
    let read = async {
        while let Some(line) = next_line(&mut output).await {
            let pid = pid.load(std::sync::atomic::Ordering::Relaxed);
            // simctl's own line, once the app ended.
            if line == format!("{id}: {pid}") {
                continue;
            }
            let mut entry = crate::log::entry(now(), LogLevel::Notice, pid, line);
            entry.process = Some(executable.clone());
            entry.tag = Some("console".to_owned());
            log.push(entry);
        }
    };
    tokio::select! {
        () = ended.cancelled() => {}
        () = read => ended.cancel(),
    }
    drop(child);
}
