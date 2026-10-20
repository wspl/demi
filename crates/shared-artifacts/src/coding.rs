//! The content coding of a command executable on its way to a runner
//! (`crates/backend-runners/src/publication.rs`): packaging compresses each executable with zstd, the object store
//! serves the compressed copy under `Content-Encoding`, and the download
//! decodes it before it verifies.

use std::io::{Read as _, Write as _};

use bytes::Bytes;
use futures_util::{Stream, TryStreamExt as _};

use crate::{Digest, Error, Verifier};

/// The `Content-Encoding` an encoded executable is served with.
pub const CONTENT_CODING: &str = "zstd";

/// The most bytes `size` bytes can be served as, in [`CONTENT_CODING`] or
/// as they are: a download of a file of known size holds no more
/// (`crates/shared-artifacts/src/download.rs`).
pub fn encoded_bound(size: u64) -> u64 {
    let size = usize::try_from(size).unwrap_or(usize::MAX);
    zstd::zstd_safe::compress_bound(size).max(size) as u64
}

/// The first bytes of a zstd frame.
pub(crate) const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

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
    decode_into(encoded, expected, &mut std::io::sink())
}

/// `encoded`, bytes in the content coding, decoded, once they are bytes of
/// `expected`'s size and SHA-256. Blocking.
pub fn decode_blocking(encoded: &[u8], expected: &Digest) -> Result<Vec<u8>, Error> {
    let size = usize::try_from(expected.size).unwrap_or(usize::MAX);
    let mut decoded = Vec::with_capacity(size);
    decode_into(encoded, expected, &mut decoded)?;
    Ok(decoded)
}

/// Decodes `encoded` into `output`, checking what it decodes to against
/// `expected` as it goes. Blocking.
fn decode_into(
    encoded: &[u8],
    expected: &Digest,
    output: &mut impl std::io::Write,
) -> Result<(), Error> {
    let mut decoder = zstd::stream::read::Decoder::new(encoded)?;
    let mut verifier = Verifier::new(expected);
    let mut buffer = vec![0; 256 * 1024];
    loop {
        let count = decoder.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        verifier.update(&buffer[..count])?;
        output.write_all(&buffer[..count])?;
    }
    verifier.finish()
}

/// `encoded`, a stream of bytes in the content coding, decoded as it
/// streams, for a reader that cannot decode it, such as an installer. What
/// it decodes to is whatever the encoded bytes hold: the caller serves bytes
/// that were checked when they were stored.
pub fn decode_stream<E>(
    encoded: impl Stream<Item = Result<Bytes, E>> + Send + Unpin,
) -> impl Stream<Item = std::io::Result<Bytes>> + Send
where
    E: std::error::Error + Send + Sync + 'static,
{
    let reader = tokio_util::io::StreamReader::new(encoded.map_err(std::io::Error::other));
    let decoder = async_compression::tokio::bufread::ZstdDecoder::new(reader);
    tokio_util::io::ReaderStream::new(decoder)
}

/// Decodes the file at `encoded`, bytes in the content coding, into
/// `output`, checking what it decodes to against `expected`, as a download
/// kept in its coding is checked once it is complete.
pub async fn decode_file(
    encoded: &std::path::Path,
    expected: &Digest,
    output: &mut (impl tokio::io::AsyncWrite + Unpin),
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), Error> {
    use tokio::io::AsyncBufReadExt as _;
    let mut file = tokio::io::BufReader::new(tokio::fs::File::open(encoded).await?);
    if !file.fill_buf().await?.starts_with(&ZSTD_MAGIC) {
        return Err(Error::Uncompressed);
    }
    let mut decoder = async_compression::tokio::bufread::ZstdDecoder::new(file);
    crate::copy(&mut decoder, expected, output, cancel).await
}
