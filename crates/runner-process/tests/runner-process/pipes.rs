use bytes::Bytes;
use demi_runner_process::pipes::PipeClient;
use demi_runner_protocol::wire;
use futures_util::StreamExt;
use std::{io, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// Only opening a connection has a deadline: an answer's headers, a body
/// and an upload each stay quiet for three times as long and still complete.
/// The quiet is real time, kept short: the deadline is the HTTP client's timer
/// on real sockets, where a paused clock would also advance while bytes are
/// in flight.
#[tokio::test]
async fn quiet_uploads_delayed_headers_and_bodies_outlive_the_connect_deadline() {
    const DEADLINE: Duration = Duration::from_millis(200);
    const QUIET: Duration = Duration::from_millis(600);
    tokio::time::timeout(Duration::from_secs(60), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let tasks = TaskTracker::new();
        let handlers = tasks.clone();
        let server = tokio::spawn(async move {
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                handlers.spawn(async move {
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        header.push(socket.read_u8().await.unwrap());
                        assert!(header.len() < 16 * 1024);
                    }
                    let text = String::from_utf8(header).unwrap();
                    assert!(text.to_lowercase().contains("authorization: bearer test-token"));
                    if text.starts_with("PUT ") {
                        let mut body = Vec::new();
                        while !body.ends_with(b"0\r\n\r\n") {
                            body.push(socket.read_u8().await.unwrap());
                            assert!(body.len() < 1024);
                        }
                        let body = String::from_utf8(body).unwrap();
                        assert!(body.contains("first"));
                        assert!(body.contains("last"));
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    } else if text.starts_with("GET /headers ") {
                        tokio::time::sleep(QUIET).await;
                        let body = [data(b"reply"), wire::end_frame(&Ok(())).to_vec()].concat();
                        socket.write_all(&response_head(body.len())).await.unwrap();
                        socket.write_all(&body).await.unwrap();
                    } else {
                        let (first, rest) = (data(b"first"), [data(b"last!"), wire::end_frame(&Ok(())).to_vec()].concat());
                        socket.write_all(&response_head(first.len() + rest.len())).await.unwrap();
                        socket.write_all(&first).await.unwrap();
                        tokio::time::sleep(QUIET).await;
                        socket.write_all(&rest).await.unwrap();
                    }
                });
            }
        });
        let token = tokio::sync::watch::Sender::new(Some("test-token".parse().unwrap())).subscribe();
        let client = PipeClient::with_connect_timeout(&origin.parse().unwrap(), token, DEADLINE).unwrap();
        let cancel = CancellationToken::new();
        let collect = async |path| {
            let mut stream = client.get(path, cancel.clone()).await.unwrap();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await { bytes.extend(chunk.unwrap()); }
            bytes
        };
        let body = futures_util::stream::unfold(0, |index| async move {
            match index {
                0 => Some((Ok::<_, io::Error>(Bytes::from_static(b"first")), 1)),
                1 => {
                    tokio::time::sleep(QUIET).await;
                    Some((Ok(Bytes::from_static(b"last")), 2))
                }
                _ => None,
            }
        });
        let (headers, response, upload) = tokio::join!(collect("/headers"), collect("/body"), client.put("/upload", body, &cancel));
        assert_eq!(headers, b"reply");
        assert_eq!(response, b"firstlast!");
        upload.unwrap();
        server.await.unwrap();
        tasks.close();
        tasks.wait().await;
    }).await.unwrap();
}

#[tokio::test]
async fn explicit_cancel_interrupts_quiet_input_and_urls_cannot_change_origin() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let client = PipeClient::new(
            &origin.parse().unwrap(),
            tokio::sync::watch::Sender::new(Some("token".parse().unwrap())).subscribe(),
        )
        .unwrap();
        for path in ["//elsewhere/pipe", "https://elsewhere/pipe", "/\\elsewhere"] {
            assert_eq!(
                client
                    .get(path, CancellationToken::new())
                    .await
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        let cancel = CancellationToken::new();
        let aborted = cancel.clone();
        let task = tokio::spawn(async move { client.get("/quiet", aborted).await });
        let (_socket, _) = listener.accept().await.unwrap();
        cancel.cancel();
        assert_eq!(
            task.await.unwrap().err().unwrap().kind(),
            io::ErrorKind::Interrupted
        );
    })
    .await
    .unwrap();
}

/// `bytes` as a pipe's data frames.
fn data(bytes: &[u8]) -> Vec<u8> {
    let mut frames = bytes::BytesMut::new();
    wire::encode_data(bytes, &mut frames);
    frames.to_vec()
}

/// The head of a pipe's answer whose body is `length` bytes.
fn response_head(length: usize) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").into_bytes()
}

/// A pipe's reader passes on only the bytes of its data frames, however the
/// frames arrive, and ends as the end frame says (`runner.md` § Pipes and
/// output): a whole pipe's end ends its bytes, a broken one's is the cause's
/// error, and a body that stops without an end frame, as an edge that cuts
/// a response does, is a broken pipe. Before, the frames reached the
/// reader's output as bytes, and a cut body read as a whole one. Each case
/// is a local server's one answer; about 0.01 s.
#[tokio::test]
async fn a_pipes_reader_passes_only_data_on_and_ends_as_its_end_frame_says() {
    let whole = [
        data(b"one "),
        wire::idle_frame().to_vec(),
        data(b"two"),
        wire::idle_frame().to_vec(),
        wire::end_frame(&Ok(())).to_vec(),
    ]
    .concat();
    let failed = [
        data(b"partial"),
        wire::end_frame(&Err("the job's device went away".into())).to_vec(),
    ]
    .concat();
    let cut = data(b"partial");
    let mut within = data(b"partial");
    within.extend_from_slice(&data(b"cut")[..4]);
    let cases: [(&[u8], &str, Result<(), &str>); 4] = [
        (&whole, "one two", Ok(())),
        (&failed, "partial", Err("the job's device went away")),
        (&cut, "partial", Err("the pipe broke off before its end: its response ended without an end frame")),
        (&within, "partial", Err("the pipe broke off before its end: its response ended within a frame")),
    ];
    tokio::time::timeout(Duration::from_secs(60), async {
        for (body, expected, end) in cases {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let body = body.to_vec();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    head.push(socket.read_u8().await.unwrap());
                }
                socket.write_all(&response_head(body.len())).await.unwrap();
                // Byte by byte, so frames arrive in pieces.
                for byte in body {
                    socket.write_all(&[byte]).await.unwrap();
                    socket.flush().await.unwrap();
                }
            });
            let token = tokio::sync::watch::Sender::new(Some("token".parse().unwrap())).subscribe();
            let client = PipeClient::new(&origin.parse().unwrap(), token).unwrap();
            let mut stream = client.get("/pipe", CancellationToken::new()).await.unwrap();
            let mut read = Vec::new();
            let mut ended = Ok(());
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => read.extend_from_slice(&bytes),
                    Err(error) => ended = Err(error.to_string()),
                }
            }
            assert_eq!(String::from_utf8(read).unwrap(), expected);
            assert_eq!(ended.as_ref().map(|_| ()).map_err(String::as_str), end);
            server.await.unwrap();
        }
    })
    .await
    .unwrap();
}

/// A stream pipe that has nothing to send pings its socket after 30 s
/// without a message, so an edge that ends a connection silent for 100 s,
/// as Cloudflare's does, keeps a still page's live view (`runner.md`
/// § Pipes and output). The clock is paused once the first message is
/// through, so the wait costs nothing; the pause comes only then, since a
/// paused clock also runs past the connection's opening deadline while its
/// bytes are in flight.
#[tokio::test]
async fn a_quiet_stream_pipe_pings_its_socket_after_30_seconds() {
    use tokio_tungstenite::tungstenite::Message;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (ended, end) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let first = socket.next().await.unwrap().unwrap();
        assert_eq!(first, Message::Binary(Bytes::from_static(b"first")));
        tokio::time::pause();
        let quiet_since = tokio::time::Instant::now();
        // An hour of the paused clock guards against a pipe that never pings.
        let ping = tokio::time::timeout(Duration::from_secs(3600), socket.next())
            .await
            .expect("a ping within the hour")
            .unwrap()
            .unwrap();
        assert!(matches!(ping, Message::Ping(_)), "{ping:?}");
        assert!(quiet_since.elapsed() >= Duration::from_secs(30), "{:?}", quiet_since.elapsed());
        drop(ended);
        // The body's end closes the socket; reading on answers the close.
        while let Some(message) = socket.next().await {
            message.unwrap();
        }
    });
    let token = tokio::sync::watch::Sender::new(Some("token".parse().unwrap())).subscribe();
    let client = PipeClient::new(&origin.parse().unwrap(), token).unwrap();
    // "first", then nothing until the server has had its ping.
    let body = futures_util::stream::iter([Ok::<_, io::Error>(Bytes::from_static(b"first"))])
        .chain(futures_util::stream::once(end).filter_map(|_| async { None }));
    let streamed = client.stream("/live", body, &CancellationToken::new()).await;
    server.await.unwrap();
    streamed.unwrap();
}
