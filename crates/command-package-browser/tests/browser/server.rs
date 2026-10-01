//! Shared local browser fixtures with deterministic stalls, downloads and submission records.
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::{
    sync::{CancellationToken, DropGuard},
    task::{AbortOnDropHandle, TaskTracker},
};

pub struct Server {
    pub base: String,
    stop: CancellationToken,
    _stop_on_drop: DropGuard,
    task: AbortOnDropHandle<()>,
}

impl Server {
    pub async fn start(fixture: &'static str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = CancellationToken::new();
        let stop_on_drop = stop.clone().drop_guard();
        let site_stop = stop.clone();
        let reloads = Arc::new(AtomicUsize::new(0));
        let submissions = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
        let typing = CancellationToken::new();
        let replacement = CancellationToken::new();
        let task = AbortOnDropHandle::new(tokio::spawn(async move {
            let connections = TaskTracker::new();
            loop {
                let accepted = tokio::select! {
                    _ = site_stop.cancelled() => break,
                    result = listener.accept() => result,
                };
                let (mut socket, _) = accepted.unwrap();
                let stopped = site_stop.clone();
                let reloads = reloads.clone();
                let submissions = submissions.clone();
                let typing = typing.clone();
                let replacement = replacement.clone();
                connections.spawn(async move {
                let work = async {
                    let mut request = Vec::new();
                    loop {
                        let mut bytes = [0; 1024];
                        let size = socket.read(&mut bytes).await?;
                        if size == 0 {
                            return Ok::<_, std::io::Error>(());
                        }
                        request.extend_from_slice(&bytes[..size]);
                        if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                            break;
                        }
                        assert!(request.len() < 16 * 1024);
                    }
                    let request = String::from_utf8(request).unwrap();
                    let path = request.split_whitespace().nth(1).unwrap();
                    if path == "/drop" || (path == "/reload-drop" && reloads.fetch_add(1, Ordering::SeqCst) > 0) {
                        return socket.shutdown().await;
                    }
                    if path == "/stall" {
                        stopped.cancelled().await;
                        return Ok(());
                    }
                    if path == "/stream-download" {
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=stream.bin\r\nContent-Length: 10485760\r\nConnection: close\r\n\r\n").await?;
                        for _ in 0..10240 {
                            socket.write_all(&[b'x'; 1024]).await?;
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        return socket.shutdown().await;
                    }
                    if path.starts_with("/submit?") { submissions.lock().await.push(path.to_owned()); }
                    if path == "/typing-started" { typing.cancel(); }
                    if path == "/wait-typing" { typing.cancelled().await; }
                    if path == "/release-load" { replacement.cancel(); }
                    if path == "/replace-load" { replacement.cancelled().await; }
                    let submitted = serde_json::to_string(&*submissions.lock().await).unwrap();
                    let (status, headers, body) = match path {
                        "/load-replaced" => ("200 OK", "", "<!doctype html><img src='/stall'><script>fetch('/replace-load').then(() => location.href='/')</script>"),
                        "/typing-started" | "/wait-typing" | "/release-load" | "/replace-load" => ("200 OK", "", "ready"),
                        "/submitted-state" => ("200 OK", "", submitted.as_str()),
                        "/frame-child" => ("200 OK", "", "<!doctype html><label>Cross frame input<input id='cross-input'></label><button id='cross-button' onclick='this.textContent=\"Cross clicked\";this.dataset.clicks=String(Number(this.dataset.clicks||0)+1);console.info(\"cross-frame console\")'>Cross button</button><iframe id='nested' srcdoc=\"<label>Nested input<input id='nested-input'></label>\"></iframe><script>window.pointerEvents=[]; for(const type of ['pointerdown','mousedown','pointerup','mouseup','click','mousemove','scroll','focusin']) document.addEventListener(type,event=>pointerEvents.push({type,target:event.target.id,x:event.clientX,y:event.clientY,scrollY,top:document.querySelector('#cross-button').getBoundingClientRect().top,at:Date.now()}),true);</script>"),
                        "/500" => ("500 Internal Server Error", "", "<!doctype html><title>HTTP error document</title><h1>Received 500</h1>"),
                        "/redirect" => ("302 Found", "Location: /destination\r\n", ""),
                        "/destination" => ("200 OK", "", "<!doctype html><title>Destination</title><h1>Arrived</h1>"),
                        _ => ("200 OK", "", fixture),
                    };
                    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len());
                    socket.write_all(response.as_bytes()).await?;
                    socket.shutdown().await
                };
                tokio::select! {
                    _ = stopped.cancelled() => {},
                    result = work => {
                        // Browser cancellation may close a fixture connection first.
                        if let Err(error) = result {
                            assert!(matches!(error.kind(), std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset), "{error}");
                        }
                    }
                }
            });
            }
            connections.close();
            connections.wait().await;
        }));
        Self {
            base,
            stop,
            _stop_on_drop: stop_on_drop,
            task,
        }
    }

    pub async fn close(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}
