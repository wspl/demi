//! A resident's log: each `tracing` event the resident writes leaves as one JSON
//! [`LogLine`] on its standard error, which the runner reads line by line and
//! keeps in the Host's log as the line it is
//! (`crates/runner-command-packages/src/process.rs`). What is not a line, a
//! panic's report for one, is kept there as text.
//!
//! A call's events carry the conversation and invocation it serves, through
//! the span the server opens around the handler (`server::invoke`).

use std::io::Write as _;

use demi_command_protocol::STDERR_LINE_BYTES;
use demi_shared_types::{LogLine, LogOrigin, log_layer::LineLayer};
use serde_json::Value;
use tracing_subscriber::{filter::LevelFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _, Layer as _};

/// Installs the resident's `tracing` subscriber: events of `info` and above
/// become lines on standard error, each naming `resident` in its fields. Every
/// resident's `main` calls it once, before it serves.
pub fn log_to_stderr(resident: &'static str) {
    log_lines_to("resident", resident.to_owned(), |bytes| {
        // A line that cannot be written has nowhere to say so: the standard
        // error is the log's only way out. One write under the lock, so that
        // threads' lines never interleave.
        let _ = std::io::stderr().lock().write_all(bytes);
    });
}

/// Installs a `tracing` subscriber whose events of `info` and above become
/// lines for the runner, each with `field` set to `value`, handed to `write`
/// as bytes with their newline. A process the runner reads other than by its
/// standard error, as a job's shell does
/// (`crates/runner-shell/src/remote/mod.rs`), sends them its own way.
pub fn log_lines_to(
    field: &'static str,
    value: String,
    write: impl Fn(&[u8]) + Send + Sync + 'static,
) {
    let layer = LineLayer::new(LogOrigin::Runner, move |mut line| {
        line.fields.insert(field.into(), value.clone().into());
        // A line that cannot be encoded has nowhere to say so.
        if let Some(bytes) = encode(&mut line) {
            write(&bytes);
        }
    });
    tracing_subscriber::registry()
        .with(layer.with_filter(LevelFilter::INFO))
        .init();
}

/// The line as bytes with its newline, at most [`STDERR_LINE_BYTES`] long: the
/// runner would split a longer one into two lines of text. The longest text
/// field is halved until it fits, then the fields go; none when even the name
/// does not fit.
fn encode(line: &mut LogLine) -> Option<Vec<u8>> {
    loop {
        let mut bytes = serde_json::to_vec(&*line).ok()?;
        bytes.push(b'\n');
        if bytes.len() <= STDERR_LINE_BYTES {
            return Some(bytes);
        }
        let halved = line
            .fields
            .values_mut()
            .filter_map(|value| match value {
                Value::String(text) => Some(text),
                _ => None,
            })
            .max_by_key(|text| text.len())
            .filter(|text| text.len() > 64)
            .map(|text| {
                let mut end = text.len() / 2;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            })
            .is_some();
        if !halved {
            if line.fields.is_empty() {
                return None;
            }
            line.fields.clear();
        }
    }
}
