//! An edge in front of the backend that holds each request's body until the
//! body ends, as a TLS edge such as Cloudflare's does, and passes a
//! WebSocket's bytes as they come (`runner.md` § Host operations). It speaks
//! HTTP/1.1, as the backend's listener does, and reads each request's head
//! and body by their framing; what the backend answers it passes on as it
//! comes.

use std::net::SocketAddr;

use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

pub struct HoldingEdge {
    /// The base URL a runner reaches the backend at through the edge.
    pub url: String,
    /// Cancelled to cut every pipe WebSocket the edge carries.
    cut: CancellationToken,
    _accepting: AbortOnDropHandle<()>,
}

impl HoldingEdge {
    /// An edge on a port of its own in front of the backend at `backend`.
    pub async fn start(backend: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let cut = CancellationToken::new();
        let cutting = cut.clone();
        let accepting = tokio::spawn(async move {
            // Each connection's task goes with the edge.
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let (client, _) = listener.accept().await.unwrap();
                connections.spawn(carry(client, backend, cutting.clone()));
            }
        });
        Self {
            url,
            cut,
            _accepting: AbortOnDropHandle::new(accepting),
        }
    }

    /// Drops the connection of every pipe WebSocket the edge carries,
    /// without a close frame, as a lost connection does.
    pub fn cut_pipe_sockets(&self) {
        self.cut.cancel();
    }
}

/// Carries one client connection to the backend: the backend's bytes back
/// as they come, each request whole once its body ended, and, after an
/// upgrade, the client's bytes as they come.
async fn carry(client: TcpStream, backend: SocketAddr, cut: CancellationToken) {
    let upstream = TcpStream::connect(backend).await.unwrap();
    let (client_read, mut client_write) = client.into_split();
    let (upstream_read, mut upstream_write) = upstream.into_split();
    let mut upstream_read = upstream_read;
    let answers = async move {
        // A connection either side closes ends here.
        let _ended = tokio::io::copy(&mut upstream_read, &mut client_write).await;
    };
    let requests = async move {
        let mut client = BufReader::new(client_read);
        loop {
            let Some(head) = read_head(&mut client).await else {
                return;
            };
            let header = |name: &str| {
                head.lines().skip(1).find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.trim()
                        .eq_ignore_ascii_case(name)
                        .then(|| value.trim().to_ascii_lowercase())
                })
            };
            if header("upgrade").is_some() {
                let pipe = head
                    .split_whitespace()
                    .nth(1)
                    .is_some_and(|path| path.starts_with("/api/pipes/"));
                upstream_write.write_all(head.as_bytes()).await.unwrap();
                let passing = async {
                    let _ended = tokio::io::copy(&mut client, &mut upstream_write).await;
                };
                if pipe {
                    tokio::select! {
                        () = cut.cancelled() => {}
                        () = passing => {}
                    }
                } else {
                    passing.await;
                }
                return;
            }
            let length = header("content-length");
            let chunked = header("transfer-encoding").is_some_and(|coding| coding.contains("chunked"));
            let mut request = head.into_bytes();
            if let Some(length) = length {
                let length: usize = length.parse().unwrap();
                let start = request.len();
                request.resize(start + length, 0);
                if client.read_exact(&mut request[start..]).await.is_err() {
                    return;
                }
            } else if chunked {
                if !read_chunks(&mut client, &mut request).await {
                    return;
                }
            }
            // The whole request, held until its body ended.
            if upstream_write.write_all(&request).await.is_err() {
                return;
            }
        }
    };
    tokio::select! {
        () = answers => {}
        () = requests => {}
    }
}

/// A request's head, its blank line included; none once the client closed.
async fn read_head(client: &mut BufReader<impl AsyncRead + Unpin>) -> Option<String> {
    let mut head = String::new();
    loop {
        let mut line = String::new();
        if client.read_line(&mut line).await.ok()? == 0 {
            return None;
        }
        head.push_str(&line);
        if line == "\r\n" {
            return Some(head);
        }
    }
}

/// Appends a chunked body to `request` as it was sent, through its last
/// chunk and its trailers; false when the client closed before its end.
async fn read_chunks(client: &mut BufReader<impl AsyncRead + Unpin>, request: &mut Vec<u8>) -> bool {
    loop {
        let mut line = String::new();
        if client.read_line(&mut line).await.unwrap_or(0) == 0 {
            return false;
        }
        request.extend_from_slice(line.as_bytes());
        let size = line.trim().split(';').next().unwrap_or_default();
        let size = usize::from_str_radix(size, 16).unwrap();
        if size == 0 {
            // The trailers, through their blank line.
            loop {
                let mut trailer = String::new();
                if client.read_line(&mut trailer).await.unwrap_or(0) == 0 {
                    return false;
                }
                request.extend_from_slice(trailer.as_bytes());
                if trailer == "\r\n" {
                    return true;
                }
            }
        }
        let start = request.len();
        request.resize(start + size + 2, 0);
        if client.read_exact(&mut request[start..]).await.is_err() {
            return false;
        }
    }
}
