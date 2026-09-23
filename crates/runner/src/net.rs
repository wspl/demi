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

use crate::{
    connection::wire::{self, NetErrorCode},
    pipes::PipeClient,
    tasks::report_pipe,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Reads feed the upload through a bounded channel, so the socket backs off
/// behind the output pipe rather than buffering without limit.
const READ_QUEUE: usize = 4;

pub struct NetStreams {
    output: mpsc::Sender<wire::Frame>,
    pipes: PipeClient,
    streams: TaskTracker,
    /// Ends every open socket: cancelled by `close` and by the host
    /// connection's shutdown.
    cancel: CancellationToken,
}

impl NetStreams {
    pub fn new(
        output: mpsc::Sender<wire::Frame>,
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
            match wire::encode(&wire::Outbound::NetOpened { stream_id }) {
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
                    tracing::warn!("net_opened encoding failed: {error}");
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

/// Resolves the host on the device and connects within 10 seconds of having a
/// socket, mapping the failure to the wire's code.
async fn connect(host: &str, port: u16) -> Result<TcpStream, (NetErrorCode, String)> {
    // `None` is a socket that could not be made for want of an open file.
    let attempt = || async {
        let addrs = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| Some((NetErrorCode::ResolveFailed, error.to_string())))?
            .collect::<Vec<_>>();
        if addrs.is_empty() {
            return Err(Some((
                NetErrorCode::ResolveFailed,
                format!("{host}:{port} resolved to no address"),
            )));
        }
        TcpStream::connect(addrs.as_slice()).await.map_err(|error| {
            if demi_command_service::descriptors::exhausted(&error) {
                return None;
            }
            Some(match error.kind() {
                io::ErrorKind::ConnectionRefused => (NetErrorCode::Refused, error.to_string()),
                _ => (NetErrorCode::Unreachable, error.to_string()),
            })
        })
    };
    // Out of open files, the socket waits for one; the ten seconds count
    // only an attempt that has a socket.
    let mut backoff = demi_command_service::descriptors::Backoff::default();
    loop {
        match tokio::time::timeout(CONNECT_TIMEOUT, attempt()).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(None)) => backoff.wait().await,
            Ok(Err(Some(failure))) => return Err(failure),
            Err(_) => {
                return Err((
                    NetErrorCode::Timeout,
                    format!("connecting to {host}:{port} exceeded 10 seconds"),
                ));
            }
        }
    }
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
    // Read errors ride the channel as items, so a reset socket fails the
    // upload instead of ending it like a clean EOF.
    let (sender, receiver) = mpsc::channel::<io::Result<Bytes>>(READ_QUEUE);
    let reads = CancellationToken::new();
    let read_cancel = reads.clone();
    let reader = tokio::spawn(async move {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            tokio::select! {
                _ = read_cancel.cancelled() => break,
                result = reader.read(&mut buffer) => match result {
                    Ok(0) => break,
                    Ok(count) => {
                        let chunk = Bytes::copy_from_slice(&buffer[..count]);
                        if sender.send(Ok(chunk)).await.is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        break;
                    }
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
                chunk = receiver.recv() => chunk.map(|chunk| (chunk, receiver)),
            }
        }
    });
    let result = pipes.put(url, body, cancel).await;
    reads.cancel();
    // A completed pipe means the read half is released: the socket is closed.
    let _ = reader.await;
    result
}

async fn send_net_error(
    output: &mpsc::Sender<wire::Frame>,
    stream_id: String,
    code: NetErrorCode,
    message: String,
    cancel: &CancellationToken,
) {
    match wire::encode(&wire::Outbound::NetError {
        stream_id,
        code,
        message,
    }) {
        Ok(message) => {
            tokio::select! {
                _ = cancel.cancelled() => {},
                _ = output.send(message) => {},
            }
        }
        Err(error) => tracing::warn!("net_error encoding failed: {error}"),
    }
}
