//! A scripted WebSocket server for the Codex WebSocket transport. It records
//! each connection's handshake headers, the messages it receives and the
//! close reason the client sends, and plays one script per connection. A
//! connection that is not a WebSocket upgrade, such as the server-sent
//! events fallback, is forwarded to a scripted HTTP vendor, so both
//! transports share one backend URL as they do in the product.

use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use http::{HeaderMap, HeaderValue, StatusCode};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{ErrorResponse, Request, Response},
};
use tokio_util::task::AbortOnDropHandle;

/// What a connection does after the client's first message.
#[derive(Clone)]
pub(crate) enum Step {
    Send(String),
    /// Closes the connection with a close frame.
    Close,
    /// Drops the connection without a close frame.
    Drop,
}

/// How a WebSocket connection answers its handshake.
#[derive(Clone)]
pub(crate) enum Script {
    /// Accepts, with `x-codex-primary-used-percent: 12` on the handshake's
    /// answer, and plays the steps.
    Accept(Vec<Step>),
    Refuse(u16),
    /// Accepts the TCP connection and drops it before any handshake.
    Drop,
}

/// What one WebSocket connection saw.
#[derive(Debug, Clone, Default)]
pub(crate) struct Connection {
    pub(crate) headers: HeaderMap,
    pub(crate) received: Vec<String>,
    pub(crate) close_reason: Option<String>,
}

type Recorded = Arc<Mutex<Vec<Connection>>>;

pub(crate) struct FakeWebSocket {
    address: SocketAddr,
    connections: Recorded,
    _server: AbortOnDropHandle<()>,
}

impl FakeWebSocket {
    /// A server playing `scripts`, one per WebSocket connection, forwarding
    /// other connections to `http`.
    pub(crate) async fn start(scripts: Vec<Script>, http: Option<SocketAddr>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let connections: Recorded = Arc::default();
        let scripts = Arc::new(Mutex::new(VecDeque::from(scripts)));
        let recorded = connections.clone();
        let server = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            while let Ok((stream, _)) = listener.accept().await {
                let scripts = scripts.clone();
                let recorded = recorded.clone();
                tasks.spawn(async move {
                    if is_upgrade(&stream).await {
                        let script = scripts.lock().unwrap().pop_front().unwrap_or(Script::Drop);
                        serve(stream, script, &recorded).await;
                    } else if let Some(http) = http {
                        forward(stream, http).await;
                    }
                });
            }
        });
        Self {
            address,
            connections,
            _server: AbortOnDropHandle::new(server),
        }
    }

    /// The backend base URL whose `…/codex/responses` this server answers.
    pub(crate) fn backend_url(&self) -> String {
        format!("http://{}/backend-api", self.address)
    }

    pub(crate) fn connections(&self) -> Vec<Connection> {
        self.connections.lock().unwrap().clone()
    }

    /// Waits until the client closed connection `index` with a reason, at
    /// most five seconds.
    pub(crate) async fn close_reason(&self, index: usize) -> Option<String> {
        for _ in 0..500 {
            if let Some(reason) = self
                .connections()
                .get(index)
                .and_then(|connection| connection.close_reason.clone())
            {
                return Some(reason);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        None
    }
}

/// Whether the connection's request asks for a WebSocket, read without
/// consuming it.
async fn is_upgrade(stream: &TcpStream) -> bool {
    let mut buffer = vec![0; 8192];
    for _ in 0..500 {
        let Ok(read) = stream.peek(&mut buffer).await else {
            return false;
        };
        let head = String::from_utf8_lossy(&buffer[..read]).to_ascii_lowercase();
        if head.contains("\r\n\r\n") || read == buffer.len() {
            return head.contains("upgrade: websocket");
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    false
}

async fn forward(mut stream: TcpStream, http: SocketAddr) {
    let Ok(mut upstream) = TcpStream::connect(http).await else {
        return;
    };
    // Either side ending ends the forwarding.
    let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
}

async fn serve(stream: TcpStream, script: Script, recorded: &Recorded) {
    let steps = match script {
        Script::Drop => {
            recorded.lock().unwrap().push(Connection::default());
            return;
        }
        Script::Refuse(status) => {
            let callback =
                move |request: &Request, _response: Response| -> Result<Response, ErrorResponse> {
                    recorded.lock().unwrap().push(Connection {
                        headers: request.headers().clone(),
                        ..Connection::default()
                    });
                    let mut refusal = ErrorResponse::new(Some("refused".into()));
                    *refusal.status_mut() = StatusCode::from_u16(status).unwrap();
                    Err(refusal)
                };
            let _ = tokio_tungstenite::accept_hdr_async(stream, callback).await;
            return;
        }
        Script::Accept(steps) => steps,
    };
    let headers = Arc::new(Mutex::new(HeaderMap::new()));
    let captured = headers.clone();
    let callback =
        move |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
            *captured.lock().unwrap() = request.headers().clone();
            response.headers_mut().insert(
                "x-codex-primary-used-percent",
                HeaderValue::from_static("12"),
            );
            Ok(response)
        };
    let Ok(mut socket) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
        return;
    };
    let index = {
        let mut connections = recorded.lock().unwrap();
        connections.push(Connection {
            headers: headers.lock().unwrap().clone(),
            ..Connection::default()
        });
        connections.len() - 1
    };
    if let Some(Ok(Message::Text(first))) = socket.next().await {
        recorded.lock().unwrap()[index]
            .received
            .push(first.as_str().to_owned());
    }
    for step in steps {
        match step {
            Step::Send(text) => {
                if socket.send(Message::text(text)).await.is_err() {
                    return;
                }
            }
            Step::Close => {
                let _ = socket.close(None).await;
                return;
            }
            Step::Drop => return,
        }
    }
    // Read until the client closes, recording its reason.
    while let Some(Ok(message)) = socket.next().await {
        if let Message::Close(frame) = message {
            let reason = frame.map_or_else(String::new, |frame| frame.reason.as_str().to_owned());
            recorded.lock().unwrap()[index].close_reason = Some(reason);
            return;
        }
    }
}
