use bytes::Bytes;
use demi_runner::pipes::PipeClient;
use futures_util::StreamExt;
use std::{io, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[tokio::test]
async fn quiet_uploads_delayed_headers_and_bodies_outlive_the_connect_deadline() {
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
                        tokio::time::sleep(Duration::from_secs(16)).await;
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nreply").await.unwrap();
                    } else {
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nfirst").await.unwrap();
                        tokio::time::sleep(Duration::from_secs(16)).await;
                        socket.write_all(b"last!").await.unwrap();
                    }
                });
            }
        });
        let client = PipeClient::new(&origin.parse().unwrap(), tokio::sync::watch::Sender::new(Some("test-token".parse().unwrap())).subscribe()).unwrap();
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
                    tokio::time::sleep(Duration::from_secs(16)).await;
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
        let client = PipeClient::new(&origin.parse().unwrap(), tokio::sync::watch::Sender::new(Some("token".parse().unwrap())).subscribe()).unwrap();
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
