//! Verified bytes (`crates-and-packages.md` § artifact): downloads over HTTPS
//! with a declared size and SHA-256, and measured ones for a release being
//! prepared, digests, durable atomic publication, release publication, the
//! install lock between processes, install receipts and archive installation.
//! Callers name the location, size and digest they expect; nothing here
//! chooses what to install.

mod archive;
mod digest;
mod download;
mod lock;
mod publish;
pub mod receipt;
mod release;

pub use archive::{Archive, install_archive, installed, zip_holds};
pub use digest::{Digest, Verifier, digest};
pub use download::{client, copy, download, download_measured};
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
    //! Test support: a fixture HTTP server on `127.0.0.1`, the client that
    //! downloads from it, and the zip archives it serves.

    use std::{
        collections::HashMap,
        io::Write as _,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    /// A download client that also allows plain HTTP, for a fixture server.
    pub fn loopback_client() -> Result<reqwest::Client, crate::Error> {
        crate::download::builder()
            .https_only(false)
            .build()
            .map_err(|error| crate::Error::Download(error.without_url().to_string()))
    }

    /// A zip archive of `entries`, each a path and its contents.
    pub fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, contents) in entries {
            writer
                .start_file(*path, zip::write::SimpleFileOptions::default())
                .expect("a fixture entry starts");
            writer.write_all(contents).expect("a fixture entry is written");
        }
        writer.finish().expect("a fixture archive ends").into_inner()
    }

    /// What a fixture server answers for one path.
    #[derive(Debug, Clone)]
    pub struct Answer {
        pub status: u16,
        pub body: Vec<u8>,
        /// Whether the answer declares its length; without one, the body
        /// ends when the connection closes.
        pub length: bool,
    }

    impl Answer {
        /// `200` with `body` and its length.
        pub fn ok(body: impl Into<Vec<u8>>) -> Self {
            Self {
                status: 200,
                body: body.into(),
                length: true,
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
                                body: Vec::new(),
                                length: true,
                            };
                            let answer = answers.get(path).unwrap_or(&missing);
                            let length = if answer.length {
                                format!("content-length: {}\r\n", answer.body.len())
                            } else {
                                String::new()
                            };
                            let head = format!("HTTP/1.1 {} Fixture\r\n{length}connection: close\r\n\r\n", answer.status);
                            // The client may hang up first, as on a failed
                            // check; nothing waits for the answer then.
                            let _written = socket.write_all(head.as_bytes()).await;
                            let _written = socket.write_all(&answer.body).await;
                        });
                    }
                }
            });
            Self { base, requests, accept }
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
    /// The archive is not what its installation needs, such as one that
    /// does not extract or lacks its executable.
    #[error("the archive {0}")]
    Archive(String),
    /// An installation in place is not the one its receipt and archive name.
    #[error("the installation at {} {reason}", .directory.display())]
    Installation { directory: std::path::PathBuf, reason: String },
    /// A release already at its directory differs from the one being
    /// published: a published release is immutable.
    #[error("{} is already published with other contents", .0.display())]
    Conflict(std::path::PathBuf),
    #[error("cancelled")]
    Cancelled,
}
