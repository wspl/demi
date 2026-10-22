//! Verified bytes.
//!
//! The crate holds:
//!
//! - downloads over HTTPS (plain HTTP too for a caller whose digest came over a
//!   connection it trusts) with a declared size and SHA-256, and measured ones
//!   for a release being prepared;
//! - digests, and durable atomic publication;
//! - release publication;
//! - the install lock between processes, install receipts and archive
//!   installation;
//! - the content coding a command executable is served in.
//!
//! Callers name the location, size and digest they expect; nothing here chooses
//! what to install.
//!
//! Every download-and-verify path and every atomic publication of a file,
//! durable or not, goes through this crate: the runner's artifact cache, the
//! Claude Code installer, the machine manager's image store, the edit
//! recorder's snapshots and journal in `command-sdk`, and `xtask`'s release
//! packaging.
//!
//! The crate also owns `WORKSPACE_VERSION`, the version in `Cargo.toml` that
//! every program and package reports.

mod archive;
mod coding;
mod digest;
mod download;
mod lock;
mod publish;
pub mod receipt;
mod release;

pub use archive::{
    Archive, ArchiveInstall, Unpacking, holds, install_archive, installed, recorded,
};
pub use coding::{
    CONTENT_CODING, Effort, check_encoded_blocking, encoded_bound, decode_blocking, decode_file, decode_stream,
    encode_blocking,
};
pub use digest::{Digest, Verifier, digest};
pub use download::{
    Download, Fetch, Headers, NO_HEADERS, Patience, Received, Unsent, client, copy, discard, download,
    download_bytes,
    download_client, download_client_allowing_http, download_client_through,
};
pub use lock::InstallLock;
pub use publish::{
    Mode, Permissions, Publication, Staged, publish, publish_bytes, publish_bytes_blocking,
    publish_directory,
};
pub use release::{ReleaseFile, ReleaseRecord, publish_release};
/// The workspace version this build carries, which every program reports
/// and a server release is named by: `Cargo.toml`'s, which a deploy too
/// commits before its build (`upgrades.md` § A development build on a
/// server).
pub const WORKSPACE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The HTTP client the downloads take, so a caller names it without a
/// dependency of its own; `client` makes the one every download uses.
pub use reqwest::Client;

/// Why verified bytes could not be had.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The IO error is the source, so a caller that waits out a lack of open
    /// files finds it.
    #[error("{0}")]
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
    /// The server sent the bytes in a content coding the download does not
    /// decode: only [`CONTENT_CODING`] and none are.
    #[error("the artifact arrived in the content coding {0}, which downloads do not decode")]
    Coding(String),
    /// Bytes kept in their coding are not compressed with zstd, as when a
    /// proxy decoded them on the way.
    #[error(
        "the download is not compressed with zstd: a proxy between this device and its backend may have decompressed it"
    )]
    Uncompressed,
    /// The archive is not what its installation needs, such as one that
    /// does not extract or lacks its executable.
    #[error("the archive {0}")]
    Archive(String),
    /// An installation in place is not the one its receipt and archive name.
    #[error("the installation at {} {reason}", .directory.display())]
    Installation {
        directory: std::path::PathBuf,
        reason: String,
    },
    /// A release already at its directory differs from the one being
    /// published: a published release is immutable.
    #[error("{} is already published with other contents", .0.display())]
    Conflict(std::path::PathBuf),
    #[error("cancelled")]
    Cancelled,
}
