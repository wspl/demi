//! The expose relay (`expose.md` § The public relay, § Acceptance): a
//! visitor's request reaches the exposed service on a real runner's device
//! as the visitor sent it, and the service's answer, streamed or upgraded,
//! comes back as the service sent it. The services are fixtures on this
//! machine, which is the device's network here. The visitor and the HTTP
//! fixture write and read raw bytes, so the header case each side wrote is
//! what the other reads.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use demi_core::Clock as _;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::io::{AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::http::HeaderMap;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_util::task::AbortOnDropHandle;

use crate::support::{Harness, Session, TestBackend};

const DOMAIN: &str = "expose.localhost";
/// The expose of the HTTP fixture.
const HTTP_EXPOSE: &str = "k7x2maqw4p3s6tavaw2y4z6aab";
/// The expose of the WebSocket fixture.
const WEBSOCKET_EXPOSE: &str = "m3n5p7rgtxv3w5x7yez4a3c5ek";
/// The size of the bodies that cross the relay in each direction.
const BODY_BYTES: usize = 8 << 20;
/// How long a step may take before the test names what never came.
const STEP: Duration = Duration::from_secs(20);

#[tokio::test]
async fn a_relayed_request_reaches_the_service_as_sent_and_its_answer_comes_back_as_sent() {
    let harness = Harness::new().with_expose_domain(DOMAIN);
    let (backend, master) = harness.start_set_up().await;
    let laptop = backend.pair(&master, "laptop").await;
    let proceed = Arc::new(Notify::new());
    let mut fixture = HttpFixture::start(proceed.clone()).await;
    expose(&harness, &master, HTTP_EXPOSE, laptop.id(), fixture.port);
    let host = format!("{HTTP_EXPOSE}.{DOMAIN}:{}", backend.address().port());

    // Header names keep their case both ways; the relay rewrites Host, adds
    // the forwarded headers and asks the service to close its connection;
    // what concerns one connection only stays behind; a repeated header
    // keeps its separate lines in order.
    let (mut read, mut write) = visit(&backend).await;
    let request = format!(
        "GET /headers?q=1 HTTP/1.1\r\nHost: {host}\r\nX-Custom-Header: One\r\nx-lower-case: two\r\n\
         X-UPPER-CASE: THREE\r\nCookie: a=1; b=2\r\nConnection: keep-alive, X-Hop\r\nKeep-Alive: timeout=5\r\n\
         X-Hop: gone\r\nX-Custom-Header: Four\r\n\r\n"
    );
    write.write_all(request.as_bytes()).await.unwrap();
    let answer = read_head(&mut read).await;
    let Seen::Request { head: seen, .. } = fixture.next().await else {
        panic!("the service saw an event stream");
    };
    assert_eq!(seen.line, "GET /headers?q=1 HTTP/1.1");
    let service_address = format!("127.0.0.1:{}", fixture.port);
    for (name, value) in [
        ("Host", service_address.as_str()),
        ("x-lower-case", "two"),
        ("X-UPPER-CASE", "THREE"),
        ("Cookie", "a=1; b=2"),
    ] {
        assert_eq!(seen.lines(name), [(name, value)], "{:?}", seen.headers);
    }
    assert_eq!(
        seen.lines("x-custom-header"),
        [("X-Custom-Header", "One"), ("X-Custom-Header", "Four")]
    );
    for (name, value) in [
        ("connection", "close"),
        ("x-forwarded-for", "127.0.0.1"),
        ("x-forwarded-host", host.as_str()),
        ("x-forwarded-proto", "http"),
    ] {
        assert_eq!(seen.values(name), [value], "{name}: {:?}", seen.headers);
    }
    assert_eq!(seen.headers.len(), 10, "nothing else reaches the service: {:?}", seen.headers);
    assert_eq!(answer.line, "HTTP/1.1 200 OK");
    for (name, value) in [
        ("Content-Type", "text/plain"),
        ("X-Service-Header", "Yes"),
        ("Content-Length", "5"),
    ] {
        assert_eq!(answer.lines(name), [(name, value)], "{:?}", answer.headers);
    }
    assert_eq!(
        answer.lines("set-cookie"),
        [("Set-Cookie", "first=1; Path=/"), ("set-cookie", "second=2; HttpOnly")]
    );
    assert!(answer.lines("connection").is_empty(), "{:?}", answer.headers);
    let mut body = [0; 5];
    read.read_exact(&mut body).await.unwrap();
    assert_eq!(&body, b"hello");

    // A chunked request body of 8 MiB reaches the service whole, and so
    // does its chunked answer of 8 MiB the visitor.
    let (mut read, mut write) = visit(&backend).await;
    let head = format!(
        "POST /upload HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/octet-stream\r\n\
         Transfer-Encoding: chunked\r\n\r\n"
    );
    write.write_all(head.as_bytes()).await.unwrap();
    write_chunked(&mut write, &pattern(BODY_BYTES, 3)).await;
    let answer = read_head(&mut read).await;
    assert_eq!(answer.line, "HTTP/1.1 200 OK");
    assert_eq!(answer.values("transfer-encoding"), ["chunked"]);
    assert!(read_chunked(&mut read).await == pattern(BODY_BYTES, 7), "the answer arrived changed");
    let Seen::Request { head: seen, body } = fixture.next().await else {
        panic!("the service saw an event stream");
    };
    assert_eq!(seen.line, "POST /upload HTTP/1.1");
    assert_eq!(seen.values("transfer-encoding"), ["chunked"]);
    assert!(body == pattern(BODY_BYTES, 3), "the request body arrived changed");

    // An event stream reaches the visitor event by event: the service sends
    // the second event only once the visitor read the first. The stream's
    // input ends only once the answer is complete.
    let (mut read, mut write) = visit(&backend).await;
    let request = format!("GET /events HTTP/1.1\r\nHost: {host}\r\nAccept: text/event-stream\r\n\r\n");
    write.write_all(request.as_bytes()).await.unwrap();
    let answer = read_head(&mut read).await;
    assert_eq!(answer.line, "HTTP/1.1 200 OK");
    assert_eq!(answer.lines("Content-Type"), [("Content-Type", "text/event-stream")]);
    let first = tokio::time::timeout(STEP, read_until(&mut read, "data: 1\n\n"))
        .await
        .expect("the first event reaches the visitor before the service sends the second");
    assert_eq!(first, "event: tick\ndata: 1\n\n");
    proceed.notify_one();
    let second = read_until(&mut read, "data: 2\n\n").await;
    assert_eq!(second, "event: tick\ndata: 2\n\n");
    assert_eq!(next_chunk(&mut read).await, None);
    assert_eq!(
        fixture.next().await,
        Seen::EventStream {
            open_while_streaming: true,
            ended_after_answer: true,
        }
    );

    // A refused upgrade reaches the visitor as the service answered it,
    // after the service saw the handshake as the visitor sent it.
    let (mut read, mut write) = visit(&backend).await;
    let request = format!(
        "GET /refuse HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    write.write_all(request.as_bytes()).await.unwrap();
    let answer = read_head(&mut read).await;
    assert_eq!(answer.line, "HTTP/1.1 426 Upgrade Required");
    assert_eq!(answer.lines("Content-Length"), [("Content-Length", "17")]);
    let mut body = [0; 17];
    read.read_exact(&mut body).await.unwrap();
    assert_eq!(&body, b"no upgrades here\n");
    let Seen::Request { head: seen, .. } = fixture.next().await else {
        panic!("the service saw an event stream");
    };
    for (name, value) in [
        ("Upgrade", "websocket"),
        ("Connection", "Upgrade"),
        ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
        ("Sec-WebSocket-Version", "13"),
    ] {
        assert_eq!(seen.lines(name), [(name, value)], "{:?}", seen.headers);
    }

    backend.close().await;
}

#[tokio::test]
async fn a_websocket_through_the_relay_carries_messages_and_close_codes_both_ways() {
    let harness = Harness::new().with_expose_domain(DOMAIN);
    let (backend, master) = harness.start_set_up().await;
    let laptop = backend.pair(&master, "laptop").await;
    let mut fixture = WebSocketFixture::start().await;
    expose(&harness, &master, WEBSOCKET_EXPOSE, laptop.id(), fixture.port);
    let url = format!("ws://{WEBSOCKET_EXPOSE}.{DOMAIN}:{}/socket", backend.address().port());

    // The visitor's handshake reaches the service with its key, and the
    // service's switch reaches the visitor.
    let stream = TcpStream::connect(backend.address()).await.unwrap();
    let (mut socket, switched) = tokio_tungstenite::client_async(url.as_str(), stream).await.unwrap();
    assert_eq!(switched.status(), 101);
    assert!(switched.headers().contains_key("sec-websocket-accept"));
    let WebSocketSeen::Handshake(handshake) = fixture.next().await else {
        panic!("the service saw no handshake");
    };
    assert_eq!(handshake["host"], format!("127.0.0.1:{}", fixture.port));
    assert!(handshake.contains_key("sec-websocket-key"), "{handshake:?}");
    // Text, binary, and a ping answered by the service's pong, unchanged.
    for message in [
        Message::text("hello"),
        Message::binary(vec![0, 1, 2, 255]),
    ] {
        socket.send(message.clone()).await.unwrap();
        assert_eq!(socket.next().await.unwrap().unwrap(), message);
    }
    socket.send(Message::Ping(Bytes::from_static(b"are you there"))).await.unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::Pong(Bytes::from_static(b"are you there"))
    );
    // The visitor's close code and reason reach the service, whose answer
    // reaches the visitor.
    let visitor_done = CloseFrame {
        code: CloseCode::from(4001),
        reason: "visitor done".into(),
    };
    socket.close(Some(visitor_done.clone())).await.unwrap();
    assert_eq!(fixture.next().await, WebSocketSeen::Closed(Some(visitor_done.clone())));
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::Close(Some(visitor_done))
    );

    // The service's close code and reason reach the visitor.
    let stream = TcpStream::connect(backend.address()).await.unwrap();
    let (mut socket, _) = tokio_tungstenite::client_async(url.as_str(), stream).await.unwrap();
    assert!(matches!(fixture.next().await, WebSocketSeen::Handshake(_)));
    socket.send(Message::text("close")).await.unwrap();
    let service_done = CloseFrame {
        code: CloseCode::from(4002),
        reason: "service done".into(),
    };
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::Close(Some(service_done))
    );

    backend.close().await;
}

/// An expose of the fixture on `port` of `device`, for an hour from the
/// backend's clock.
fn expose(harness: &Harness, master: &Session, id: &str, device: &str, port: u16) {
    let now = harness.clock.now().as_millisecond();
    harness
        .control_database()
        .execute(
            "INSERT INTO exposes (id, user_id, device_id, address, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                id,
                master.user.id.as_str(),
                device,
                format!("127.0.0.1:{port}"),
                now,
                now + 3_600_000
            ],
        )
        .unwrap();
}

/// A visitor's connection to the backend, in raw bytes.
async fn visit(backend: &TestBackend) -> (BufReader<OwnedReadHalf>, OwnedWriteHalf) {
    let (read, write) = TcpStream::connect(backend.address()).await.unwrap().into_split();
    (BufReader::new(read), write)
}

/// `length` bytes of a pattern that `seed` varies.
fn pattern(length: usize, seed: u8) -> Vec<u8> {
    (0..length)
        .map(|index| u8::try_from(index % 251).unwrap() ^ seed)
        .collect()
}

/// An HTTP message's head as its bytes had it: the first line, and each
/// header line's name and value in their order, names in their case.
#[derive(Debug, PartialEq)]
struct Head {
    line: String,
    headers: Vec<(String, String)>,
}

impl Head {
    /// The lines of `name`, whatever its case, in their order.
    fn lines(&self, name: &str) -> Vec<(&str, &str)> {
        self.headers
            .iter()
            .filter(|(line, _)| line.eq_ignore_ascii_case(name))
            .map(|(line, value)| (line.as_str(), value.as_str()))
            .collect()
    }

    /// The values of `name`, whatever its case, in their order.
    fn values(&self, name: &str) -> Vec<&str> {
        self.lines(name).into_iter().map(|(_, value)| value).collect()
    }
}

/// Reads a message head, up to and with the empty line.
async fn read_head(read: &mut (impl AsyncBufRead + Unpin)) -> Head {
    let mut bytes = Vec::new();
    loop {
        let start = bytes.len();
        let count = read.read_until(b'\n', &mut bytes).await.unwrap();
        assert!(count > 0, "the stream ended in a head: {}", String::from_utf8_lossy(&bytes));
        if &bytes[start..] == b"\r\n" {
            break;
        }
    }
    let first = bytes.iter().position(|byte| *byte == b'\n').unwrap() + 1;
    let line = String::from_utf8(bytes[..first - 2].to_vec()).unwrap();
    let mut slots = [httparse::EMPTY_HEADER; 64];
    let httparse::Status::Complete((_, parsed)) = httparse::parse_headers(&bytes[first..], &mut slots).unwrap() else {
        panic!("an incomplete head: {}", String::from_utf8_lossy(&bytes));
    };
    let headers = parsed
        .iter()
        .map(|header| (header.name.to_owned(), String::from_utf8(header.value.to_vec()).unwrap()))
        .collect();
    Head { line, headers }
}

/// The next chunk of a chunked body; none at its last chunk, whose
/// trailers are read too.
async fn next_chunk(read: &mut (impl AsyncBufRead + Unpin)) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    read.read_until(b'\n', &mut line).await.unwrap();
    let httparse::Status::Complete((_, size)) = httparse::parse_chunk_size(&line).unwrap() else {
        panic!("no chunk size: {}", String::from_utf8_lossy(&line));
    };
    if size == 0 {
        loop {
            let mut trailer = Vec::new();
            read.read_until(b'\n', &mut trailer).await.unwrap();
            if trailer == b"\r\n" {
                return None;
            }
        }
    }
    let mut chunk = vec![0; usize::try_from(size).unwrap()];
    read.read_exact(&mut chunk).await.unwrap();
    let mut end = [0; 2];
    read.read_exact(&mut end).await.unwrap();
    assert_eq!(&end, b"\r\n");
    Some(chunk)
}

/// A chunked body, whole.
async fn read_chunked(read: &mut (impl AsyncBufRead + Unpin)) -> Vec<u8> {
    let mut body = Vec::new();
    while let Some(chunk) = next_chunk(read).await {
        body.extend(chunk);
    }
    body
}

/// Chunks of a chunked body until their text ends with `end`, as text.
async fn read_until(read: &mut (impl AsyncBufRead + Unpin), end: &str) -> String {
    let mut text = String::new();
    while !text.ends_with(end) {
        let chunk = next_chunk(read).await.expect("the body goes on");
        text.push_str(std::str::from_utf8(&chunk).unwrap());
    }
    text
}

/// Writes `body` as a chunked body, in chunks of 64 KiB.
async fn write_chunked(write: &mut (impl AsyncWrite + Unpin), body: &[u8]) {
    for chunk in body.chunks(64 << 10) {
        write.write_all(format!("{:x}\r\n", chunk.len()).as_bytes()).await.unwrap();
        write.write_all(chunk).await.unwrap();
        write.write_all(b"\r\n").await.unwrap();
    }
    write.write_all(b"0\r\n\r\n").await.unwrap();
}

/// What the HTTP fixture saw of one request.
#[derive(Debug, PartialEq)]
enum Seen {
    Request {
        head: Head,
        body: Vec<u8>,
    },
    /// An event stream's answer, and whether its connection's input stayed
    /// open while it streamed and ended after it.
    EventStream {
        open_while_streaming: bool,
        ended_after_answer: bool,
    },
}

/// A service on this machine that reads and writes raw bytes, by path:
/// `/headers` answers with headers of mixed case and two cookies,
/// `/upload` reads a chunked body whole and answers `BODY_BYTES` chunked,
/// `/events` streams two events, the second once `proceed` is notified,
/// and `/refuse` refuses an upgrade.
struct HttpFixture {
    port: u16,
    seen: mpsc::UnboundedReceiver<Seen>,
    _serving: AbortOnDropHandle<()>,
}

impl HttpFixture {
    async fn start(proceed: Arc<Notify>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen, receiver) = mpsc::unbounded_channel();
        let serving = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                tokio::spawn(serve_http(socket, seen.clone(), proceed.clone()));
            }
        });
        Self {
            port,
            seen: receiver,
            _serving: AbortOnDropHandle::new(serving),
        }
    }

    async fn next(&mut self) -> Seen {
        let seen = tokio::time::timeout(STEP, self.seen.recv()).await;
        seen.expect("the service sees the request").unwrap()
    }
}

async fn serve_http(socket: TcpStream, seen: mpsc::UnboundedSender<Seen>, proceed: Arc<Notify>) {
    let (read, mut write) = socket.into_split();
    let mut read = BufReader::new(read);
    let head = read_head(&mut read).await;
    let target = head.line.split(' ').nth(1).unwrap().to_owned();
    match target.split('?').next().unwrap() {
        "/headers" => {
            seen.send(Seen::Request { head, body: Vec::new() }).unwrap();
            let answer = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Service-Header: Yes\r\n\
                          Set-Cookie: first=1; Path=/\r\nset-cookie: second=2; HttpOnly\r\nContent-Length: 5\r\n\
                          Connection: close\r\n\r\nhello";
            write.write_all(answer.as_bytes()).await.unwrap();
        }
        "/upload" => {
            let body = read_chunked(&mut read).await;
            seen.send(Seen::Request { head, body }).unwrap();
            let head = "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
            write.write_all(head.as_bytes()).await.unwrap();
            write_chunked(&mut write, &pattern(BODY_BYTES, 7)).await;
        }
        "/events" => {
            // Many servers abort an answer still streaming once the
            // client's side of the connection ends, so it must end last.
            let mut ended = tokio::spawn(async move {
                let mut byte = [0; 1];
                // Its end, a failure or a stray byte all end the watch.
                let _ = read.read(&mut byte).await;
            });
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\
                        Transfer-Encoding: chunked\r\n\r\n";
            write.write_all(head.as_bytes()).await.unwrap();
            write.write_all(chunk("event: tick\ndata: 1\n\n").as_bytes()).await.unwrap();
            proceed.notified().await;
            let open_while_streaming = !ended.is_finished();
            write.write_all(chunk("event: tick\ndata: 2\n\n").as_bytes()).await.unwrap();
            write.write_all(b"0\r\n\r\n").await.unwrap();
            let ended_after_answer = tokio::time::timeout(STEP, &mut ended).await.is_ok();
            seen.send(Seen::EventStream {
                open_while_streaming,
                ended_after_answer,
            })
            .unwrap();
        }
        "/refuse" => {
            seen.send(Seen::Request { head, body: Vec::new() }).unwrap();
            let answer = "HTTP/1.1 426 Upgrade Required\r\nContent-Type: text/plain\r\nContent-Length: 17\r\n\
                          Connection: close\r\n\r\nno upgrades here\n";
            write.write_all(answer.as_bytes()).await.unwrap();
        }
        other => panic!("the fixture serves no {other}"),
    }
}

/// `text` as one chunk of a chunked body.
fn chunk(text: &str) -> String {
    format!("{:x}\r\n{text}\r\n", text.len())
}

/// What the WebSocket fixture saw.
#[derive(Debug, PartialEq)]
enum WebSocketSeen {
    /// A handshake's request headers.
    Handshake(HeaderMap),
    /// A close frame the visitor sent.
    Closed(Option<CloseFrame>),
}

/// A WebSocket service on this machine: it echoes text and binary
/// messages, answers pings, and closes with 4002 when the visitor sends the
/// text `close`.
struct WebSocketFixture {
    port: u16,
    seen: mpsc::UnboundedReceiver<WebSocketSeen>,
    _serving: AbortOnDropHandle<()>,
}

impl WebSocketFixture {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen, receiver) = mpsc::unbounded_channel();
        let serving = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                tokio::spawn(serve_websocket(socket, seen.clone()));
            }
        });
        Self {
            port,
            seen: receiver,
            _serving: AbortOnDropHandle::new(serving),
        }
    }

    async fn next(&mut self) -> WebSocketSeen {
        let seen = tokio::time::timeout(STEP, self.seen.recv()).await;
        seen.expect("the service sees the visitor").unwrap()
    }
}

async fn serve_websocket(socket: TcpStream, seen: mpsc::UnboundedSender<WebSocketSeen>) {
    let handshake = seen.clone();
    let record = move |request: &Request, response: Response| {
        handshake.send(WebSocketSeen::Handshake(request.headers().clone())).unwrap();
        Ok(response)
    };
    let mut socket = tokio_tungstenite::accept_hdr_async(socket, record).await.unwrap();
    while let Some(Ok(message)) = socket.next().await {
        match message {
            Message::Text(text) if text.as_str() == "close" => {
                let done = CloseFrame {
                    code: CloseCode::from(4002),
                    reason: "service done".into(),
                };
                socket.close(Some(done)).await.unwrap();
            }
            Message::Text(_) | Message::Binary(_) => socket.send(message).await.unwrap(),
            Message::Close(frame) => seen.send(WebSocketSeen::Closed(frame)).unwrap(),
            // tungstenite answers pings itself.
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}
