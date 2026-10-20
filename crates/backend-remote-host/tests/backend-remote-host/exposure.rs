//! The backend's requests to a device's exposed port over a real runner
//! (`crates/backend-user-shard/src/shard/exposed_ports.rs`): the runner's connection to the
//! port lasts as long as the request's caller waits, however long the port
//! takes, and an answer the relay cannot carry fails with why.
//!
//! Each test drives a local server of its own that tells the test what
//! happens on its connection; a test that waits on one ends within its
//! budget instead of hanging when the runner gets it wrong.

use std::time::Duration;

use demi_backend_remote_host::{
    HttpRequest, PipeReader,
    testing::{FixtureOptions, RunnerFixture, TEST_DEVICE},
};
use demi_runner_protocol::exposure::{HttpHeader, HttpScheme};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

/// The most any one wait of these tests takes when the runner does right.
const BUDGET: Duration = Duration::from_secs(10);

/// A server on a local port that takes one connection and reads the
/// request's head.
struct Port {
    listener: TcpListener,
    port: u16,
}

impl Port {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        Self { listener, port }
    }

    /// The connection and the request's head, as the port read them.
    async fn request(&self) -> (TcpStream, String) {
        let (mut connection, _) = within(self.listener.accept()).await.unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            within(connection.read_exact(&mut byte)).await.unwrap();
            head.push(byte[0]);
        }
        (connection, String::from_utf8(head).unwrap())
    }
}

/// `future`, which must end within the budget.
async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(BUDGET, future)
        .await
        .expect("the runner did this within the budget")
}

/// Waits until the runner closes `connection`.
async fn closed(mut connection: TcpStream) {
    let mut rest = Vec::new();
    within(connection.read_to_end(&mut rest)).await.unwrap();
}

fn get(port: u16, headers: Vec<HttpHeader>) -> HttpRequest {
    HttpRequest {
        scheme: HttpScheme::Http,
        port,
        method: "GET".into(),
        path: "/v1/models".into(),
        headers,
    }
}

fn header(name: &str, value: &str) -> HttpHeader {
    HttpHeader {
        name: name.into(),
        value: value.into(),
    }
}

/// The answer's body, as the caller reads it.
async fn body(reader: PipeReader) -> Result<Vec<u8>, String> {
    let mut reader = reader;
    let mut body = Vec::new();
    while let Some(chunk) = within(reader.next()).await {
        body.extend_from_slice(&chunk.map_err(|failure| failure.to_string())?);
    }
    Ok(body)
}

#[tokio::test(flavor = "local")]
async fn a_caller_that_stops_waiting_before_the_answer_closes_the_runners_connection_to_the_port() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let link = fixture.link().await;
    let port = Port::new().await;
    let answer = fixture.pipes().from_device(TEST_DEVICE);
    let reader = answer.reader().unwrap().failing_when_dropped("the caller stopped waiting");
    let request = link.http_request(get(port.port, Vec::new()), None, answer.wire_ref());
    let (connection, _) = tokio::select! {
        _ = request => panic!("the port never answers"),
        asked = port.request() => asked,
    };
    // The request went out and the port thinks; its caller stops waiting.
    drop(reader);
    closed(connection).await;
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn a_caller_that_stops_reading_an_answer_under_way_closes_the_runners_connection_to_the_port() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let link = fixture.link().await;
    let port = Port::new().await;
    let answer = fixture.pipes().from_device(TEST_DEVICE);
    let reader = answer.reader().unwrap().failing_when_dropped("the caller stopped reading");
    let request = link.http_request(get(port.port, Vec::new()), None, answer.wire_ref());
    tokio::pin!(request);
    let (mut connection, _) = tokio::select! {
        _ = &mut request => panic!("the port has not answered yet"),
        asked = port.request() => asked,
    };
    // The answer's head and its first token; the rest never comes.
    connection
        .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n5\r\nhello\r\n")
        .await
        .unwrap();
    let head = within(request).await.unwrap();
    assert_eq!(head.status, 200);
    drop(reader);
    closed(connection).await;
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn a_port_that_answers_after_a_pipes_arrival_window_is_waited_for() {
    // The arrival window is the time a pipe gives its ends to arrive; the
    // port answers only after a pipe minted later than the request's
    // answer pipe has run out of it.
    let fixture = RunnerFixture::start(FixtureOptions {
        arrival: Some(Duration::from_millis(300)),
        ..FixtureOptions::default()
    })
    .await;
    let link = fixture.link().await;
    let port = Port::new().await;
    let answer = fixture.pipes().from_device(TEST_DEVICE);
    let reader = answer.reader().unwrap();
    let request = link.http_request(get(port.port, Vec::new()), None, answer.wire_ref());
    tokio::pin!(request);
    let (mut connection, _) = tokio::select! {
        _ = &mut request => panic!("the port has not answered yet"),
        asked = port.request() => asked,
    };
    let window = fixture.pipes().from_device(TEST_DEVICE);
    let _waits = window.reader().unwrap();
    tokio::select! {
        _ = &mut request => panic!("the port has not answered yet"),
        _ = within(window.failed()) => {}
    }
    connection
        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nhello")
        .await
        .unwrap();
    let head = within(request).await.unwrap();
    assert_eq!(head.status, 200);
    assert_eq!(body(reader).await.unwrap(), b"hello");
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn an_answer_with_more_headers_than_the_relay_carries_fails_the_request_with_why() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let link = fixture.link().await;
    let port = Port::new().await;
    let answer = fixture.pipes().from_device(TEST_DEVICE);
    let reader = answer.reader().unwrap();
    let request = link.http_request(get(port.port, Vec::new()), None, answer.wire_ref());
    tokio::pin!(request);
    let (mut connection, _) = tokio::select! {
        _ = &mut request => panic!("the port has not answered yet"),
        asked = port.request() => asked,
    };
    let mut head = String::from("HTTP/1.1 200 OK\r\ncontent-length: 0\r\n");
    // 256 more, so the answer has 257.
    for number in 0..256 {
        head.push_str(&format!("x-header-{number}: {number}\r\n"));
    }
    head.push_str("\r\n");
    connection.write_all(head.as_bytes()).await.unwrap();
    let refused = within(request).await.unwrap_err();
    assert!(refused.message.contains("257 headers"), "{}", refused.message);
    // The pipe is released with it, never left waiting.
    let failed = body(reader).await.unwrap_err();
    assert!(failed.contains("257 headers"), "{failed}");
    fixture.stop().await;
}

#[tokio::test(flavor = "local")]
async fn the_headers_a_connection_header_names_stay_on_their_hop_both_ways() {
    let fixture = RunnerFixture::start(FixtureOptions::default()).await;
    let link = fixture.link().await;
    let port = Port::new().await;
    let answer = fixture.pipes().from_device(TEST_DEVICE);
    let reader = answer.reader().unwrap();
    let headers = vec![
        header("connection", "x-hop"),
        header("x-hop", "the backend's own"),
        header("x-end", "kept"),
    ];
    let request = link.http_request(get(port.port, headers), None, answer.wire_ref());
    tokio::pin!(request);
    let (mut connection, asked) = tokio::select! {
        _ = &mut request => panic!("the port has not answered yet"),
        asked = port.request() => asked,
    };
    let asked = asked.to_ascii_lowercase();
    assert!(asked.contains("x-end: kept"), "{asked}");
    assert!(!asked.contains("x-hop:"), "{asked}");
    connection
        .write_all(b"HTTP/1.1 200 OK\r\nconnection: x-private\r\nx-private: 1\r\nx-public: 1\r\ncontent-length: 0\r\n\r\n")
        .await
        .unwrap();
    let head = within(request).await.unwrap();
    let names: Vec<&str> = head.headers.iter().map(|header| header.name.as_str()).collect();
    assert!(names.contains(&"x-public"), "{names:?}");
    assert!(!names.contains(&"x-private"), "{names:?}");
    assert!(!names.contains(&"connection"), "{names:?}");
    body(reader).await.unwrap();
    fixture.stop().await;
}
