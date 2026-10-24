//! The commands' default text: short and readable, the screen's text quoted
//! with its control characters escaped, so a label cannot forge a line of
//! the result.

use std::fmt::Write;

use demi_shared_types::output::{Cell, Table};

use demi_command_package_mobile_protocol::mobile::{
    DeviceId, Element, ElementSummary, Frame, InspectResult, LOG_BYTES, LaunchResult, ListResult, LogEntry, LogLevel,
    LogsResult, Orientation, MobilePlatform, PowerState, Simulator,
};


/// `text` quoted, as JSON quotes a string.
pub fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}

pub fn orientation(orientation: Orientation) -> &'static str {
    match orientation {
        Orientation::Portrait => "portrait",
        Orientation::LandscapeLeft => "landscape left",
        Orientation::PortraitUpsideDown => "portrait upside down",
        Orientation::LandscapeRight => "landscape right",
    }
}

/// A simulator's state as `list` prints it.
fn state(simulator: &Simulator) -> &'static str {
    if simulator.open.is_some() {
        return "open";
    }
    if simulator.elsewhere {
        return "open elsewhere";
    }
    match simulator.state {
        PowerState::ShutDown => "shut down",
        PowerState::Booting => "booting",
        PowerState::Booted => "booted",
        PowerState::ShuttingDown => "shutting down",
    }
}

fn platform(platform: MobilePlatform) -> &'static str {
    match platform {
        MobilePlatform::Ios => "ios",
        MobilePlatform::Android => "android",
    }
}

/// `list`: a row per simulator, its `simctl` id or AVD name first, as
/// `open` takes it, and its name last; the handle while this conversation
/// has it open. Why a platform has none goes to standard error.
pub fn list(result: &ListResult) -> String {
    let mut table = Table::new(["id", "platform", "os", "state", "handle", "model", "name"]);
    for simulator in &result.devices {
        table.row::<Cell>([
            simulator.native_id.as_str().into(),
            platform(simulator.platform).into(),
            simulator.os.as_str().into(),
            state(simulator).into(),
            simulator.open.as_ref().map(ToString::to_string).into(),
            Some(simulator.model.as_str()).filter(|model| !model.is_empty()).into(),
            simulator.name.as_str().into(),
        ]);
    }
    table.render()
}

fn frame(frame: &Frame) -> String {
    format!("[{},{} {}×{}]", frame.x, frame.y, frame.width, frame.height)
}

/// An element as an action's acknowledgement names it, as the tree names
/// it: `button "Sign Up" [ref=e7]`.
pub fn summary(element: &ElementSummary) -> String {
    match &element.label {
        Some(label) => format!("{} {} [ref={}]", element.role, quoted(label), element.r#ref),
        None => format!("{} [ref={}]", element.role, element.r#ref),
    }
}

pub fn inspect(result: &InspectResult, name: &str) -> String {
    let mut text = format!(
        "Device: {} · {name} · {} · {}×{}\n\n",
        result.device,
        orientation(result.orientation),
        result.width,
        result.height
    );
    for element in &result.elements {
        tree(&mut text, element, 0);
    }
    if result.elements.is_empty() {
        text.push_str("The screen shows no elements.\n");
    }
    text
}

fn tree(text: &mut String, element: &Element, depth: usize) {
    let _ = write!(text, "{}- {}", "  ".repeat(depth), element.role);
    if let Some(label) = &element.label {
        let _ = write!(text, " {}", quoted(label));
    }
    let _ = write!(text, " [ref={}]", element.r#ref);
    if let Some(value) = &element.value
        && Some(value) != element.label.as_ref()
    {
        let _ = write!(text, " value={}", quoted(value));
    }
    if let Some(identifier) = &element.identifier
        && Some(identifier) != element.label.as_ref()
    {
        let _ = write!(text, " id={}", quoted(identifier));
    }
    for (flag, word) in [
        (element.enabled == Some(false), "[disabled]"),
        (element.selected == Some(true), "[selected]"),
        (element.focused == Some(true), "[focused]"),
        (element.protected == Some(true), "[protected]"),
    ] {
        if flag {
            let _ = write!(text, " {word}");
        }
    }
    let _ = writeln!(text, " {}", frame(&element.frame));
    for child in &element.children {
        tree(text, child, depth + 1);
    }
}

pub fn installed(device: &DeviceId, app: &str, version: Option<&str>) -> String {
    match version {
        Some(version) => format!("Installed {} {} on {device}.\n", plain(app), plain(version)),
        None => format!("Installed {} on {device}.\n", plain(app)),
    }
}

pub fn launched(result: &LaunchResult) -> String {
    let app = plain(&result.app);
    let mut text = if result.started {
        format!("Launched {app} on {}, in front.\n", result.device)
    } else {
        format!("Brought {app} in front on {}; it was running.\n", result.device)
    };
    if !result.shown {
        text.push_str(
            "Its first screen has not shown yet, as no element is on it; wait --element waits for one, screenshot shows what is there.\n",
        );
    }
    if !result.console {
        text.push_str(
            "What it prints is not in logs, as Demi did not start it; launch --restart starts it again with its console.\n",
        );
    }
    text
}

/// `logs`: a line per entry, its local time, its level's letter, where it
/// comes from and its text, as `logcat` prints a line; then the cursor.
pub fn logs(result: &LogsResult) -> String {
    let mut text = String::new();
    if result.dropped > 0 {
        let _ = writeln!(
            text,
            "{} entries after the cursor are gone: the log keeps its last {} MB.",
            result.dropped,
            LOG_BYTES / (1024 * 1024)
        );
    }
    let zone = jiff::tz::TimeZone::system();
    for entry in &result.entries {
        let _ = writeln!(text, "{} {} {}: {}", time(entry, &zone), letter(entry.level), source(entry), plain(&entry.text));
    }
    if result.entries.is_empty() {
        text.push_str("No log entries.\n");
    }
    let _ = writeln!(text, "Cursor: {}", plain(&result.cursor));
    if result.has_more {
        text.push_str("Has more: true\n");
    }
    text
}

fn time(entry: &LogEntry, zone: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_millisecond(entry.timestamp as i64)
        .map(|timestamp| timestamp.to_zoned(zone.clone()).strftime("%H:%M:%S%.3f").to_string())
        .unwrap_or_else(|_| "--:--:--.---".into())
}

/// Each level's letter, as `logcat` writes Android's, with N for notice.
fn letter(level: LogLevel) -> char {
    match level {
        LogLevel::Verbose => 'V',
        LogLevel::Debug => 'D',
        LogLevel::Info => 'I',
        LogLevel::Notice => 'N',
        LogLevel::Warning => 'W',
        LogLevel::Error => 'E',
        LogLevel::Fault => 'F',
    }
}

/// Where an entry comes from: its iOS process and its tag.
fn source(entry: &LogEntry) -> String {
    let words: Vec<String> = [entry.process.as_deref(), entry.tag.as_deref()].into_iter().flatten().map(plain).collect();
    if words.is_empty() { format!("[{}]", entry.pid) } else { words.join(" ") }
}

/// `text` on one line: its control characters escaped, so an entry cannot
/// forge another, nor the cursor.
fn plain(text: &str) -> String {
    text.chars()
        .flat_map(|c| -> Box<dyn Iterator<Item = char>> {
            if c.is_control() { Box::new(c.escape_default()) } else { Box::new(std::iter::once(c)) }
        })
        .collect()
}
