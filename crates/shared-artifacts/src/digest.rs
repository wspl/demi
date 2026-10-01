//! A file's size and SHA-256, and the check of bytes against a declared one.

use std::{io::Read, path::Path};

use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

use crate::Error;

/// A file's size and its SHA-256 in lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Digest {
    pub size: u64,
    pub sha256: String,
}

/// Counts and hashes bytes as they arrive; bytes past the limit fail at once.
#[derive(Debug)]
pub(crate) struct Measure {
    hash: Sha256,
    size: u64,
    limit: u64,
}

impl Measure {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            hash: Sha256::new(),
            size: 0,
            limit,
        }
    }

    pub(crate) fn update(&mut self, chunk: &[u8]) -> Result<(), Error> {
        self.size = self
            .size
            .checked_add(chunk.len() as u64)
            .filter(|size| *size <= self.limit)
            .ok_or(Error::TooLarge { declared: self.limit })?;
        self.hash.update(chunk);
        Ok(())
    }

    /// The size and SHA-256 of the bytes seen.
    pub(crate) fn finish(self) -> Digest {
        Digest {
            size: self.size,
            sha256: format!("{:x}", self.hash.finalize()),
        }
    }
}

/// Checks bytes against a declared digest as they arrive, so a download that
/// grows past its size stops at once.
#[derive(Debug)]
pub struct Verifier {
    expected: Digest,
    measure: Measure,
}

impl Verifier {
    pub fn new(expected: &Digest) -> Self {
        Self {
            expected: expected.clone(),
            measure: Measure::new(expected.size),
        }
    }

    /// Counts and hashes the next chunk; bytes past the declared size fail.
    pub fn update(&mut self, chunk: &[u8]) -> Result<(), Error> {
        self.measure.update(chunk)
    }

    /// Whether the bytes seen are exactly the declared ones.
    pub fn finish(self) -> Result<(), Error> {
        let seen = self.measure.finish();
        if seen.size != self.expected.size {
            return Err(Error::Size {
                declared: self.expected.size,
                actual: seen.size,
            });
        }
        if seen.sha256 != self.expected.sha256 {
            return Err(Error::Digest);
        }
        Ok(())
    }
}

/// Hashes the file at `path` on the blocking pool; a file longer than `limit`
/// bytes fails without being read to its end.
pub async fn digest(path: &Path, limit: u64, cancel: &CancellationToken) -> Result<Digest, Error> {
    let path = path.to_owned();
    let cancel = cancel.clone();
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::open(path)?;
        let mut measure = Measure::new(limit);
        let mut buffer = vec![0; 64 * 1024];
        loop {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            measure.update(&buffer[..count])?;
        }
        Ok(measure.finish())
    })
    .await
    .map_err(std::io::Error::other)?
}
