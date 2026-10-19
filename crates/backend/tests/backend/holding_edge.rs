//! An edge in front of the backend that holds each request's body until the
//! body ends, as a TLS edge such as Cloudflare's does, and passes a
//! WebSocket's bytes as they come (`runner.md` § Host operations). It speaks
//! HTTP/1.1, as the backend's listener does, and reads each request's head
//! and body by their framing; what the backend answers it passes on as it
//! comes.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

pub struct HoldingEdge {
    /// The base URL a runner reaches the backend at through the edge.
    pub url: String,
    /// Cancelled to cut every pipe WebSocket the edge carries.
    cut: CancellationToken,
    /// The body of each pipe upload the edge carried: a runner sends a
    /// file's bytes to the backend with one (`runner.md` § File contents).
    pipe_uploads: Arc<Mutex<Vec<Vec<u8>>>>,
    _accepting: AbortOnDropHandle<()>,
}

impl HoldingEdge {
    /// An edge on a port of its own in front of the backend at `backend`.
    pub async fn start(backend: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let cut = CancellationToken::new();
        let cutting = cut.clone();
        let pipe_uploads = Arc::new(Mutex::new(Vec::new()));
        let counting = pipe_uploads.clone();
        let accepting = tokio::spawn(async move {
            // Each connection's task goes with the edge.
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let (client, _) = listener.accept().await.unwrap();
                connections.spawn(carry(client, backend, cutting.clone(), counting.clone()));
            }
        });
        Self {
            url,
            cut,
            pipe_uploads,
            _accepting: AbortOnDropHandle::new(accepting),
        }
    }

    /// Drops the connection of every pipe WebSocket the edge carries,
    /// without a close frame, as a lost connection does.
    pub fn cut_pipe_sockets(&self) {
        self.cut.cancel();
    }

    /// How many pipe uploads the edge carried so far.
    /// How many of the pipe uploads the edge carried hold `bytes`.
    pub fn pipe_uploads_of(&self, bytes: &[u8]) -> usize {
        let uploads = self.pipe_uploads.lock().unwrap();
        uploads
            .iter()
            .filter(|body| body.windows(bytes.len()).any(|window| window == bytes))
            .count()
    }
}

/// Carries one client connection to the backend: the backend's bytes back
/// as they come, each request whole once its body ended, and, after an
/// upgrade, the client's bytes as they come.
async fn carry(
    client: TcpStream,
    backend: SocketAddr,
    cut: CancellationToken,
    pipe_uploads: Arc<Mutex<Vec<Vec<u8>>>>,
) {
    let upstream = TcpStream::connect(backend).await.unwrap();
    let (client_read, mut client_write) = client.into_split();
    let (upstream_read, mut upstream_write) = upstream.into_split();
    let mut upstream_read = upstream_read;
    // How many of the backend's bytes the edge passed on, which tells a
    // pipe that opened, its answer passed, from one still opening.
    let (answered, passed) = tokio::sync::watch::channel(0_usize);
    let answers = async move {
        // A connection either side closes ends here.
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            let Ok(read) = upstream_read.read(&mut buffer).await else {
                return;
            };
            if read == 0 || client_write.write_all(&buffer[..read]).await.is_err() {
                return;
            }
            answered.send_modify(|bytes| *bytes += read);
        }
    };
    let requests = async move {
        let mut client = BufReader::new(client_read);
        loop {
            let Some(head) = read_head(&mut client).await else {
                return;
            };
            let pipe_upload = head.starts_with("PUT /api/pipes/");
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
                let before = *passed.borrow();
                upstream_write.write_all(head.as_bytes()).await.unwrap();
                let passing = async {
                    let _ended = tokio::io::copy(&mut client, &mut upstream_write).await;
                };
                if pipe {
                    // A cut drops a pipe once it is open, the backend's
                    // answer passed, as a connection lost partway does; one
                    // dropped before its answer would be a pipe that never
                    // opened, which the runner opens again.
                    let mut passed = passed.clone();
                    let open_then_cut = async {
                        // An edge whose backend went away cuts nothing more.
                        if passed.wait_for(|bytes| *bytes > before).await.is_ok() {
                            cut.cancelled().await;
                        }
                    };
                    tokio::select! {
                        () = open_then_cut => {}
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
            let mut body = Vec::new();
            if let Some(length) = length {
                let length: usize = length.parse().unwrap();
                let start = request.len();
                request.resize(start + length, 0);
                if client.read_exact(&mut request[start..]).await.is_err() {
                    return;
                }
                body.extend_from_slice(&request[start..]);
            } else if chunked {
                if !read_chunks(&mut client, &mut request, &mut body).await {
                    return;
                }
            }
            if pipe_upload {
                pipe_uploads.lock().unwrap().push(body);
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
/// chunk and its trailers, and its chunks' data to `body`; false when the
/// client closed before its end.
async fn read_chunks(
    client: &mut BufReader<impl AsyncRead + Unpin>,
    request: &mut Vec<u8>,
    body: &mut Vec<u8>,
) -> bool {
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
        body.extend_from_slice(&request[start..start + size]);
    }
}
