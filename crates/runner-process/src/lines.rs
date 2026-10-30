//! The split of a stream into lines, for the Host log: the runner's own
//! diagnostics, a resident's standard error and a job's shell's lines.

/// A longer line of text continues on the next line.
pub const LINE_BYTES: usize = demi_command_protocol::STDERR_LINE_BYTES;

/// Cuts a byte stream that arrives in arbitrary chunks into log lines: at
/// each newline, and at `LINE_BYTES` when no newline comes.
#[derive(Default)]
pub struct LineSplitter {
    pending: Vec<u8>,
}

impl LineSplitter {
    /// The lines `chunk` completes; an unfinished line waits for the next chunk.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for byte in chunk {
            if *byte == b'\n' {
                lines.extend(self.take());
                continue;
            }
            self.pending.push(*byte);
            if self.pending.len() == LINE_BYTES {
                lines.extend(self.take());
            }
        }
        lines
    }

    /// What the stream left without a final newline.
    pub fn finish(mut self) -> Option<String> {
        self.take()
    }

    fn take(&mut self) -> Option<String> {
        let bytes = std::mem::take(&mut self.pending);
        let text = String::from_utf8_lossy(&bytes);
        let text = text.trim_end_matches('\r');
        if text.is_empty() {
            return None;
        }
        Some(text.to_owned())
    }
}

/// Hands one line another of Demi's processes wrote to the Host's log: a
/// resident's standard error, or a job's shell (`crates/runner-shell/src/remote/mod.rs`).
/// Such a process writes its `tracing` events as JSON lines
/// (`demi_command_sdk::log_to_stderr`), which the log keeps as they are, in the
/// `resident_line` field that `crates/runner/src/host_log.rs` reads; any other
/// line is kept as text, as a panic's report is.
pub fn log_line(source: &str, line: &str) {
    use demi_runner_protocol::wire;
    let parsed = line
        .starts_with('{')
        .then(|| serde_json::from_str::<wire::LogLine>(line).ok())
        .flatten()
        .filter(|parsed| wire::is_event_name(&parsed.event));
    let Some(parsed) = parsed else {
        tracing::info!(source, "{line}");
        return;
    };
    match parsed.level {
        wire::LogLevel::Trace => tracing::trace!(resident_line = line, "{}", parsed.event),
        wire::LogLevel::Debug => tracing::debug!(resident_line = line, "{}", parsed.event),
        wire::LogLevel::Info => tracing::info!(resident_line = line, "{}", parsed.event),
        wire::LogLevel::Warn => tracing::warn!(resident_line = line, "{}", parsed.event),
        wire::LogLevel::Error => tracing::error!(resident_line = line, "{}", parsed.event),
    }
}
