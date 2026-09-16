//! TCP streams the backend opens on the device's network (`runner.md`
//! § Network streams): connect, then carry bytes between the socket and the
//! two pipes the request named.

use std::{io, time::Duration};

use bytes::Bytes;
use futures_util::StreamExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{connection::wire, pipes::PipeClient, tasks::report_pipe};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Reads feed the upload through a bounded channel, so the socket backs off
/// behind the output pipe rather than buffering without limit.
const READ_QUEUE: usize = 4;

pub struct NetStreams {
    output: mpsc::Sender<wire::Outbound>,
    pipes: PipeClient,
    streams: TaskTracker,
    /// Ends every open socket: cancelled by `close` and by the host
    /// connection's shutdown.
    cancel: CancellationToken,
}

impl NetStreams {
    pub fn new(
        output: mpsc::Sender<wire::Outbound>,
        pipes: PipeClient,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            output,
            pipes,
            streams: TaskTracker::new(),
            cancel,
        }
    }

    /// Starts one stream; connection cancellation (or `close`) ends every
    /// open socket.
    pub fn handle_open(&self, message: wire::Inbound) -> io::Result<()> {
        let wire::Inbound::NetOpen {
            stream_id,
            host,
            port,
            input,
            output,
        } = message
        else {
            return Err(io::Error::other("not a net_open request"));
        };
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("host connection closed"));
        }
        let port = u16::try_from(port)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "net_open port is invalid"))?;
        let pipes = self.pipes.clone();
        let reply = self.output.clone();
        let reporting = self.cancel.clone();
        let stream = self.cancel.child_token();
        self.streams.spawn(async move {
            let socket = match connect(&host, port).await {
                Ok(socket) => socket,
                Err((code, message)) => {
                    send_net_error(&reply, stream_id, code, message, &stream).await;
                    return;
                }
            };
            // No bytes move before the answer.
            match wire::net_opened(stream_id) {
                Ok(message) => {
                    tokio::select! {
                        _ = stream.cancelled() => return,
                        result = reply.send(message) => {
                            if result.is_err() {
                                // The connection owner has closed its receiver.
                                return;
                            }
                        }
                    }
                }
                Err(error) => {
                    eprintln!("demi-runner: net_opened encoding failed: {error}");
                    return;
                }
            }
            // The two directions share one socket: either direction failing
            // cancels the stream token, which stops the other and drops both
            // halves, closing the socket.
            let (reader, writer) = socket.into_split();
            let exchange = TaskTracker::new();
            // Input pipe → socket write side; input EOF shuts that side.
            exchange.spawn({
                let pipes = pipes.clone();
                let reply = reply.clone();
                let cancel = stream.clone();
                let failure = stream.clone();
                let reporting = reporting.clone();
                async move {
                    let result = pump_input(pipes, &input.url, writer, &cancel).await;
                    if result.is_err() {
                        failure.cancel();
                    }
                    report_pipe(&reply, input.id, result, &reporting).await;
                }
            });
            // Socket read side → output pipe; socket EOF ends the body.
            exchange.spawn({
                let reply = reply.clone();
                let cancel = stream.clone();
                let failure = stream.clone();
                let reporting = reporting.clone();
                async move {
                    let result = pump_output(pipes, &output.url, reader, &cancel).await;
                    if result.is_err() {
                        failure.cancel();
                    }
                    report_pipe(&reply, output.id, result, &reporting).await;
                }
            });
            exchange.close();
            exchange.wait().await;
        });
        Ok(())
    }

    pub async fn close(&self) {
        self.cancel.cancel();
        self.streams.close();
        self.streams.wait().await;
    }
}

/// Resolves the host on the device and connects within 10 seconds, mapping
/// the failure to the wire's code.
async fn connect(host: &str, port: u16) -> Result<TcpStream, (&'static str, String)> {
    let connect = async {
        let addrs = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| ("resolve_failed", error.to_string()))?
            .collect::<Vec<_>>();
        TcpStream::connect(addrs.as_slice())
            .await
            .map_err(|error| match error.kind() {
                io::ErrorKind::ConnectionRefused => ("refused", error.to_string()),
                _ => ("unreachable", error.to_string()),
            })
    };
    tokio::time::timeout(CONNECT_TIMEOUT, connect).await.unwrap_or_else(|_| {
        Err((
            "timeout",
            format!("connecting to {host}:{port} exceeded 10 seconds"),
        ))
    })
}

async fn pump_input(
    pipes: PipeClient,
    url: &str,
    mut writer: tokio::net::tcp::OwnedWriteHalf,
    cancel: &CancellationToken,
) -> io::Result<()> {
    let mut body = pipes.get(url, cancel.clone()).await?;
    while let Some(bytes) = body.next().await {
        let bytes = bytes?;
        tokio::select! {
            _ = cancel.cancelled() => return Err(io::Error::new(io::ErrorKind::Interrupted, "net stream cancelled")),
            result = writer.write_all(&bytes) => result?,
        }
    }
    // Input EOF is the socket's half-close of the write side.
    writer.shutdown().await
}

async fn pump_output(
    pipes: PipeClient,
    url: &str,
    mut reader: tokio::net::tcp::OwnedReadHalf,
    cancel: &CancellationToken,
) -> io::Result<()> {
    let cancel = cancel.clone();
    let (sender, receiver) = mpsc::channel::<Bytes>(READ_QUEUE);
    let reads = CancellationToken::new();
    let read_cancel = reads.clone();
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            tokio::select! {
                _ = read_cancel.cancelled() => break,
                result = reader.read(&mut buffer) => match result {
                    Ok(0) => break,
                    Ok(count) => {
                        if sender.send(Bytes::copy_from_slice(&buffer[..count])).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
            }
        }
    });
    let body_cancel = cancel.clone();
    let body = futures_util::stream::unfold(receiver, move |mut receiver| {
        let cancel = body_cancel.clone();
        async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                chunk = receiver.recv() => chunk.map(|chunk| (Ok(chunk), receiver)),
            }
        }
    });
    let result = pipes.put(url, body, &cancel).await;
    reads.cancel();
    result
}

async fn send_net_error(
    output: &mpsc::Sender<wire::Outbound>,
    stream_id: String,
    code: &'static str,
    message: String,
    cancel: &CancellationToken,
) {
    match wire::net_error(stream_id, code.to_string(), message) {
        Ok(message) => {
            tokio::select! {
                _ = cancel.cancelled() => {},
                _ = output.send(message) => {},
            }
        }
        Err(error) => eprintln!("demi-runner: net_error encoding failed: {error}"),
    }
}
