//! Verified bytes (`crates-and-packages.md` § artifact): downloads over HTTPS
//! with a declared size and SHA-256, digests, durable atomic publication, the
//! install lock between processes, install receipts and archive extraction.
//! Callers name the location, size and digest they expect; nothing here
//! chooses what to install.

mod archive;
mod digest;
mod download;
mod lock;
mod publish;
pub mod receipt;

pub use archive::extract_zip;
pub use digest::{Digest, Verifier, digest};
pub use download::{client, copy, download};
pub use lock::InstallLock;
pub use publish::{Mode, Permissions, Publication, Staged, publish, publish_bytes, publish_directory};

#[cfg(feature = "testing")]
pub mod testing {
    //! Test support: downloads from a fixture server on `127.0.0.1`.

    /// A download client that also allows plain HTTP, for a fixture server.
    pub fn loopback_client() -> Result<reqwest::Client, crate::Error> {
        crate::download::builder()
            .https_only(false)
            .build()
            .map_err(|error| crate::Error::Download(error.without_url().to_string()))
    }
}

/// Why verified bytes could not be had.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The request failed; the message leaves out the URL, which may carry
    /// a signature.
    #[error("{0}")]
    Download(String),
    /// The server answered with a status other than success.
    #[error("the server answered {status}")]
    Rejected { status: u16 },
    #[error("the artifact has more than its declared {declared} bytes")]
    TooLarge { declared: u64 },
    #[error("the artifact has {actual} bytes, not the declared {declared}")]
    Size { declared: u64, actual: u64 },
    #[error("the artifact does not match its declared SHA-256")]
    Digest,
    #[error("the archive cannot be extracted: {0}")]
    Archive(String),
    #[error("cancelled")]
    Cancelled,
}
