//! A forwarded TCP connection over a data channel of a peer between two runners
//! (`crates/runner-direct/src/pairs.rs`).
//!
//! - The Listen On runner opens the channel with the forward's header.
//! - The Connect To runner connects to the forward's address.
//! - Each side carries its TCP connection's bytes both ways.
//! - A side whose program ended its writing says `{ end: true }`, and the other
//!   ends its own writing to its program, as TCP's half-close does.
//! - The channel closes once both sides have ended.

use bytes::BytesMut;
use demi_runner_protocol::direct::{MESSAGE_BYTES, WriteEnd};
use demi_runner_protocol::forward::{DEADLINE, ForwardHeader};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use str0m::channel::ChannelId;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::operation::{Input, Operations, Output};

/// A forwarded connection's channel, as a byte stream both ways.
pub struct ForwardChannel {
    pub(crate) input: mpsc::UnboundedReceiver<Input>,
    pub(crate) output: mpsc::Sender<Output>,
    pub(crate) took: Took,
    /// Cancelled once the channel goes.
    pub(crate) cancel: CancellationToken,
    /// Closes the channel at once, cancelled when the connection ends
    /// before both its sides ended cleanly.
    pub(crate) abort: CancellationToken,
}

/// Where a consumer says how many bytes it took, which its channel grants
/// back to the other side.
#[derive(Clone)]
pub(crate) struct Took {
    pub(crate) id: ChannelId,
    pub(crate) sender: mpsc::UnboundedSender<(ChannelId, usize)>,
}

impl Took {
    fn report(&self, bytes: usize) {
        // A peer that ended grants nothing more.
        let _ = self.sender.send((self.id, bytes));
    }
}

/// The one line a forwarded connection ends with, written when it is
/// dropped, whichever way it ends, a cancelled one included.
struct Ending {
    began: Instant,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    /// The outcome and its cause.
    how: Mutex<(&'static str, &'static str)>,
}

impl Ending {
    fn new() -> Self {
        Self {
            began: Instant::now(),
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            how: Mutex::new(("cancelled", "channel_gone")),
        }
    }

    fn ended(&self, outcome: &'static str, cause: &'static str) {
        *self.how.lock().expect("the ending's lock is intact") = (outcome, cause);
    }
}

impl Drop for Ending {
    fn drop(&mut self) {
        // `bytes_in` came from the TCP connection, `bytes_out` went to it.
        let (outcome, cause) = *self.how.lock().expect("the ending's lock is intact");
        tracing::info!(
            event = "forwarded_connection.ended",
            outcome,
            cause,
            bytes_in = self.bytes_in.load(Ordering::Relaxed),
            bytes_out = self.bytes_out.load(Ordering::Relaxed),
            ms = self.began.elapsed().as_millis() as u64,
            "a forwarded connection ended"
        );
    }
}

impl ForwardChannel {
    /// Carries `tcp`'s bytes both ways until both sides ended, either side
    /// failed or the channel went; then the channel closes.
    pub async fn splice(self, tcp: TcpStream) {
        let Self {
            mut input,
            output,
            took,
            cancel,
            abort,
        } = self;
        // A connection that ends otherwise than by both sides' clean end,
        // here or by its caller going, closes its channel at once, rather
        // than once what it holds drained, which a program that stopped
        // reading never lets happen.
        let aborting = abort.drop_guard();
        let ending = Ending::new();
        let ending = &ending;
        let (mut reader, mut writer) = tcp.into_split();
        let reading = async {
            let mut buffer = BytesMut::with_capacity(MESSAGE_BYTES);
            loop {
                buffer.reserve(MESSAGE_BYTES);
                let read = reader.read_buf(&mut buffer).await;
                match read {
                    Ok(0) => {
                        let end = serde_json::to_string(&WriteEnd { end: true }).expect("an end serializes");
                        let sent = output.send(Output::Text(end)).await.is_ok();
                        if !sent {
                            ending.ended("failed", "channel_closed");
                        }
                        return sent;
                    }
                    Ok(_) => {
                        let chunk = buffer.split().freeze();
                        let bytes = chunk.len() as u64;
                        if output.send(Output::Binary(chunk)).await.is_err() {
                            ending.ended("failed", "channel_closed");
                            return false;
                        }
                        ending.bytes_in.fetch_add(bytes, Ordering::Relaxed);
                    }
                    Err(error) => {
                        tracing::debug!(
                            event = "forwarded_connection.read_failed",
                            %error,
                            "a forwarded connection could not be read"
                        );
                        ending.ended("failed", "read_failed");
                        return false;
                    }
                }
            }
        };
        let writing = async {
            while let Some(message) = input.recv().await {
                match message {
                    Input::Bytes(bytes) => {
                        if let Err(error) = writer.write_all(&bytes).await {
                            tracing::debug!(
                                event = "forwarded_connection.write_failed",
                                %error,
                                "a forwarded connection could not be written"
                            );
                            ending.ended("failed", "write_failed");
                            return false;
                        }
                        ending.bytes_out.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                        took.report(bytes.len());
                    }
                    Input::End => {
                        // A program that went already needs no end.
                        let _ = writer.shutdown().await;
                        return true;
                    }
                    Input::Watch(_) | Input::Watched(_) => {}
                }
            }
            // The channel went before the other side ended its writing.
            ending.ended("failed", "channel_closed");
            false
        };
        tokio::pin!(reading, writing);
        let (mut read, mut written) = (false, false);
        while !(read && written) {
            tokio::select! {
                () = cancel.cancelled() => return,
                ended = &mut reading, if !read => {
                    if !ended {
                        return;
                    }
                    read = true;
                }
                ended = &mut writing, if !written => {
                    if !ended {
                        return;
                    }
                    written = true;
                }
            }
        }
        ending.ended("completed", "both_ended");
        aborting.disarm();
    }
}

/// The Connect To's connection for a forwarded connection: its TCP
/// connection to the forward's address, and what ends it, cancelled when
/// the forward is turned off, removed or changed.
pub struct Forwarded {
    pub tcp: TcpStream,
    pub ended: CancellationToken,
}

/// Connects to a forward's target at `address`: refused when it does not
/// accept within [`DEADLINE`], or once `ended` is cancelled.
pub async fn connect_target(address: &str, ended: &CancellationToken) -> std::io::Result<TcpStream> {
    give_up_on(TcpStream::connect(address), ended).await
}

/// `connecting`, a connection to a forward's target, given up when it does
/// not finish within [`DEADLINE`], or once `ended` is cancelled.
pub async fn give_up_on<T>(
    connecting: impl Future<Output = std::io::Result<T>>,
    ended: &CancellationToken,
) -> std::io::Result<T> {
    tokio::select! {
        () = ended.cancelled() => Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "the forward ended")),
        connected = tokio::time::timeout(DEADLINE, connecting) => connected
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "the target did not accept in time"))?,
    }
}

/// Carries out a forwarded connection that the runner of `from` opened:
/// the runner connects as the forward's Connect To, and the channel carries
/// the connection; a refused one closes the channel at once, as a refused
/// TCP connection does.
pub(crate) async fn serve(
    operations: Arc<dyn Operations>,
    from: String,
    header: ForwardHeader,
    channel: ForwardChannel,
) {
    let ForwardHeader::Forward { forward } = header;
    // A channel that goes while the target is connected to ends the try.
    let gone = channel.cancel.clone();
    let connected = tokio::select! {
        () = gone.cancelled() => return,
        connected = operations.forward(forward, from) => connected,
    };
    match connected {
        Ok(Forwarded { tcp, ended }) => {
            tokio::select! {
                () = ended.cancelled() => {}
                () = channel.splice(tcp) => {}
            }
        }
        Err(error) => tracing::info!(
            event = "forwarded_connection.ended",
            outcome = "refused",
            cause = "target_refused",
            reason = error.message.as_str(),
            "a forwarded connection was refused"
        ),
    }
}
