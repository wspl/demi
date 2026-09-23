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

/// Checks bytes against a declared digest as they arrive, so a download that
/// grows past its size stops at once.
#[derive(Debug)]
pub struct Verifier {
    expected: Digest,
    hash: Sha256,
    size: u64,
}

impl Verifier {
    pub fn new(expected: &Digest) -> Self {
        Self {
            expected: expected.clone(),
            hash: Sha256::new(),
            size: 0,
        }
    }

    /// Counts and hashes the next chunk; bytes past the declared size fail.
    pub fn update(&mut self, chunk: &[u8]) -> Result<(), Error> {
        self.size = self
            .size
            .checked_add(chunk.len() as u64)
            .filter(|size| *size <= self.expected.size)
            .ok_or(Error::TooLarge {
                declared: self.expected.size,
            })?;
        self.hash.update(chunk);
        Ok(())
    }

    /// Whether the bytes seen are exactly the declared ones.
    pub fn finish(self) -> Result<(), Error> {
        if self.size != self.expected.size {
            return Err(Error::Size {
                declared: self.expected.size,
                actual: self.size,
            });
        }
        if format!("{:x}", self.hash.finalize()) != self.expected.sha256 {
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
        let mut hash = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = vec![0; 64 * 1024];
        loop {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            size = size
                .checked_add(count as u64)
                .filter(|size| *size <= limit)
                .ok_or(Error::TooLarge { declared: limit })?;
            hash.update(&buffer[..count]);
        }
        Ok(Digest {
            size,
            sha256: format!("{:x}", hash.finalize()),
        })
    })
    .await
    .map_err(std::io::Error::other)?
}
