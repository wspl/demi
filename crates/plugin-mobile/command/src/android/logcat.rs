//! Android's log as `logcat -v epoch,uid,usec` prints it, a line per line
//! of a message:
//!
//! ```text
//!   1792866312.866442 10213  4284  4284 I ImeTracker: onCancelled
//! ```
//!
//! the time in seconds since the epoch, the user id, the process and the
//! thread, the level's letter, and the tag before the message.

use demi_command_package_mobile_protocol::mobile::{LogEntry, LogLevel};

/// The entry of `line`; none for the lines logcat adds itself, such as the
/// start of a buffer.
pub fn entry(line: &str) -> Option<LogEntry> {
    let mut rest = line.trim_start();
    let mut field = || {
        let (word, after) = rest.split_once(char::is_whitespace)?;
        rest = after.trim_start();
        Some(word)
    };
    let seconds: f64 = field()?.parse().ok()?;
    // A user id logcat names, as root, is no app's.
    let uid = field()?.parse().ok();
    let pid = field()?.parse().ok()?;
    let _thread = field()?;
    let level = match field()? {
        "V" => LogLevel::Verbose,
        "D" => LogLevel::Debug,
        "I" => LogLevel::Info,
        "W" => LogLevel::Warning,
        "E" => LogLevel::Error,
        "F" | "A" => LogLevel::Fault,
        _ => return None,
    };
    // The tag is padded to its column before the colon.
    let (tag, text) = match rest.split_once(": ") {
        Some((tag, text)) => (tag, text),
        None => (rest.strip_suffix(':').unwrap_or(rest), ""),
    };
    let mut entry = crate::log::entry(seconds * 1000.0, level, pid, text.to_owned());
    entry.uid = uid;
    entry.tag = Some(tag.trim_end().to_owned()).filter(|tag| !tag.is_empty());
    Some(entry)
}
