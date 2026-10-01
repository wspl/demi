//! The content coding of a command executable on its way to a runner
//! (`native-runtime.md` § Publish artifacts before enabling commands): object
//! storage and the development store serve the executable encoded with zstd
//! under `Content-Encoding`, and the download decodes it before it verifies.

use std::io::Write as _;

use crate::Error;

/// The `Content-Encoding` an encoded executable is served with.
pub const CONTENT_CODING: &str = "zstd";

/// How hard an encoding works for its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    /// A published executable: encoded once and downloaded by every device,
    /// so zstd's slowest ordinary level (19) pays for itself. A 24 MB release
    /// executable takes about a second on every core and becomes 28% of its
    /// size.
    Published,
    /// A development store's executable, encoded at every backend start and
    /// often a debug build of over 100 MB: a fast level (3), a few tenths of
    /// a second, for a third of its size.
    Development,
}

impl Effort {
    fn level(self) -> i32 {
        match self {
            Self::Published => 19,
            Self::Development => 3,
        }
    }
}

/// `bytes` in the content coding, encoded with `effort` on every core.
/// Blocking.
pub fn encode_blocking(bytes: &[u8], effort: Effort) -> Result<Vec<u8>, Error> {
    let workers = std::thread::available_parallelism().map_or(1, |count| count.get());
    let mut encoder =
        zstd::stream::Encoder::new(Vec::with_capacity(bytes.len() / 3), effort.level())?;
    encoder.multithread(u32::try_from(workers).unwrap_or(u32::MAX))?;
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}
