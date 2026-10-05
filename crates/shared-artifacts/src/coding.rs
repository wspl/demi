//! The content coding of a command executable on its way to a runner
//! (`native-runtime.md` § Publish artifacts before enabling commands):
//! packaging compresses each executable with zstd, the object store serves
//! the compressed copy under `Content-Encoding`, and the download decodes it
//! before it verifies.

use std::io::{Read as _, Write as _};

use crate::{Digest, Error, Verifier};

/// The `Content-Encoding` an encoded executable is served with.
pub const CONTENT_CODING: &str = "zstd";

/// How hard an encoding works for its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    /// A released executable: encoded once, kept, and downloaded by every
    /// device, so zstd's slowest ordinary level (19) pays for itself. A
    /// 24 MB release executable takes about a second on every core and
    /// becomes 28% of its size.
    Published,
    /// A copy made often and kept briefly, such as a test's of a debug build
    /// of over 100 MB: a fast level (3), a few tenths of a second, for a
    /// third of its size.
    Fast,
}

impl Effort {
    fn level(self) -> i32 {
        match self {
            Self::Published => 19,
            Self::Fast => 3,
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

/// Checks that `encoded`, bytes in the content coding, decodes to bytes of
/// `expected`'s size and SHA-256, as a runner's download will. Blocking.
pub fn check_encoded_blocking(encoded: &[u8], expected: &Digest) -> Result<(), Error> {
    let mut decoder = zstd::stream::read::Decoder::new(encoded)?;
    let mut verifier = Verifier::new(expected);
    let mut buffer = vec![0; 256 * 1024];
    loop {
        let count = decoder.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        verifier.update(&buffer[..count])?;
    }
    verifier.finish()
}
