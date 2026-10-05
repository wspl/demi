//! Verified bytes (`crates-and-packages.md` § shared-artifacts): downloads over HTTPS
//! (plain HTTP too for a caller whose digest came over a connection it
//! trusts) with a declared size and SHA-256, and measured ones for a release
//! being prepared, digests, durable atomic publication, release publication,
//! the install lock between processes, install receipts and archive
//! installation, and the content coding a command executable is served in.
//! Callers name the location, size and digest they expect; nothing here
//! chooses what to install.

mod archive;
mod coding;
mod digest;
mod download;
mod lock;
mod publish;
pub mod receipt;
mod release;

pub use archive::{
    Archive, ArchiveInstall, Unpacking, install_archive, installed, recorded, zip_holds,
};
pub use coding::{CONTENT_CODING, Effort, check_encoded_blocking, decode_blocking, encode_blocking};
pub use digest::{Digest, Verifier, digest};
pub use download::{
    client, client_allowing_http, client_following_redirects, copy, download, download_measured,
};
pub use lock::InstallLock;
pub use publish::{
    Mode, Permissions, Publication, Staged, publish, publish_bytes, publish_bytes_blocking,
    publish_directory,
};
pub use release::{ReleaseFile, ReleaseRecord, publish_release};
/// The HTTP client the downloads take, so a caller names it without a
/// dependency of its own; `client` makes the one every download uses.
pub use reqwest::Client;

#[cfg(feature = "testing")]
pub mod testing {
    //! Test support: a fixture HTTP server on `127.0.0.1` and the zip
    //! archives it serves, which [`crate::client_allowing_http`] downloads
    //! from, the count of the process's waits for install locks, and the
    //! installation of a release a test was given unpacked.

    pub use crate::archive::install_unpacked;

    use std::{
        collections::HashMap,
        io::Write as _,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    /// Every acquisition of an install lock in this process that found the
    /// lock held.
    static LOCK_WAITS: AtomicUsize = AtomicUsize::new(0);

    pub(crate) fn count_lock_wait() {
        LOCK_WAITS.fetch_add(1, Ordering::SeqCst);
    }

    /// How many acquisitions of an install lock in this process have found it
    /// held and waited ([`crate::InstallLock`]). Nothing else shows that an
    /// installer waits for another instead of installing beside it, so a test
    /// of concurrent installers watches this count.
    pub fn lock_waits() -> usize {
        LOCK_WAITS.load(Ordering::SeqCst)
    }

    /// A zip archive of `entries`, each a path and its contents.
    pub fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, contents) in entries {
            writer
                .start_file(*path, zip::write::SimpleFileOptions::default())
                .expect("a fixture entry starts");
            writer
                .write_all(contents)
                .expect("a fixture entry is written");
        }
        writer
            .finish()
            .expect("a fixture archive ends")
            .into_inner()
    }

    /// What a fixture server answers for one path.
    #[derive(Debug, Clone)]
    pub struct Answer {
        pub status: u16,
        pub body: Vec<u8>,
        /// Whether the answer declares its length; without one, the body
        /// ends when the connection closes.
        pub length: bool,
        /// How long the server waits after the request before it answers.
        pub delay: Duration,
        /// The `Content-Encoding` the answer declares, if any.
        pub coding: Option<&'static str>,
    }

    impl Answer {
        /// `200` with `body` and its length, at once.
        pub fn ok(body: impl Into<Vec<u8>>) -> Self {
            Self {
                status: 200,
                body: body.into(),
                length: true,
                delay: Duration::ZERO,
                coding: None,
            }
        }
    }

    /// A fixture HTTP server on `127.0.0.1`: it answers each path it was
    /// given with that path's answer and any other with `404`, closes each
    /// connection after its answer, and counts the requests. Dropping it
    /// stops it.
    pub struct Server {
        base: String,
        requests: Arc<AtomicUsize>,
        accept: tokio::task::JoinHandle<()>,
    }

    impl Server {
        pub async fn start(answers: impl IntoIterator<Item = (String, Answer)>) -> Self {
            let answers: Arc<HashMap<String, Answer>> = Arc::new(answers.into_iter().collect());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("a fixture server binds");
            let base = format!("http://{}", listener.local_addr().expect("a bound address"));
            let requests = Arc::new(AtomicUsize::new(0));
            let accept = tokio::spawn({
                let requests = requests.clone();
                async move {
                    while let Ok((mut socket, _)) = listener.accept().await {
                        let answers = answers.clone();
                        let requests = requests.clone();
                        tokio::spawn(async move {
                            let mut request = vec![0; 4096];
                            // A connection that fails before its request
                            // gets no answer.
                            let Ok(read) = socket.read(&mut request).await else {
                                return;
                            };
                            requests.fetch_add(1, Ordering::SeqCst);
                            // The request line is `GET <path> HTTP/1.1`.
                            let head = String::from_utf8_lossy(&request[..read]);
                            let path = head.split_whitespace().nth(1).unwrap_or_default();
                            let missing = Answer {
                                status: 404,
                                ..Answer::ok(Vec::new())
                            };
                            let answer = answers.get(path).unwrap_or(&missing);
                            tokio::time::sleep(answer.delay).await;
                            let length = if answer.length {
                                format!("content-length: {}\r\n", answer.body.len())
                            } else {
                                String::new()
                            };
                            let coding = answer
                                .coding
                                .map(|coding| format!("content-encoding: {coding}\r\n"))
                                .unwrap_or_default();
                            let head = format!(
                                "HTTP/1.1 {} Fixture\r\n{length}{coding}connection: close\r\n\r\n",
                                answer.status
                            );
                            // The client may hang up first, as on a failed
                            // check; nothing waits for the answer then.
                            let _written = socket.write_all(head.as_bytes()).await;
                            let _written = socket.write_all(&answer.body).await;
                        });
                    }
                }
            });
            Self {
                base,
                requests,
                accept,
            }
        }

        /// The URL of `path` on this server.
        pub fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base)
        }

        /// How many requests the server has read.
        pub fn requests(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.accept.abort();
        }
    }
}

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
