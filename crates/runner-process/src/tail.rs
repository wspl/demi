//! The last bytes of a stream within a limit: a service's or an invocation's
//! standard error, and the view of a job's output.

/// Keeps the last `limit` bytes pushed into it.
#[derive(Debug)]
pub struct TailBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl TailBuffer {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) {
        let chunk = &chunk[chunk.len().saturating_sub(self.limit)..];
        let excess = (self.bytes.len() + chunk.len()).saturating_sub(self.limit);
        self.bytes.drain(..excess);
        self.bytes.extend_from_slice(chunk);
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// The bytes as text. A character the limit cut in two loses only itself.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}
