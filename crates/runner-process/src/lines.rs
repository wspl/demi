//! The split of a stream into lines, for the Host log: the runner's own
//! diagnostics and a service's standard error.

/// A longer line of text continues on the next line.
pub const LINE_BYTES: usize = 4096;

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
