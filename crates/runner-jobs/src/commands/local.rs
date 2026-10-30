//! The runner's private local endpoint (`crates/runner-process/src/command_client.rs`), which command aliases and the runner's management commands
//! reach.

use demi_command_protocol::LocalInvocation;
use demi_command_sdk::{Handler, ResidentError};
use demi_runner_process::local_socket::Listener;
use std::io;
use std::sync::Arc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// Counts one open client connection while it lives.
struct ActiveConnection(watch::Sender<usize>);

impl ActiveConnection {
    fn new(active: &watch::Sender<usize>) -> Self {
        active.send_modify(|count| *count += 1);
        Self(active.clone())
    }
}

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count -= 1);
    }
}

pub struct Server {
    endpoint: String,
    cancel: CancellationToken,
    owner: Option<tokio::task::JoinHandle<io::Result<()>>>,
    /// How many client connections are open.
    active: watch::Sender<usize>,
}

impl Server {
    /// Serves `handler` at the endpoint of the installation in `installation`,
    /// whose address stays the same from one start of its runner to the
    /// next, so that a program a job started before reaches the runner that
    /// runs now (`crates/runner-shell/src/remote/mod.rs`).
    pub async fn start(
        handler: Arc<dyn Handler<Metadata = LocalInvocation>>,
        installation: &std::path::Path,
    ) -> io::Result<Self> {
        // A system where the named address cannot be, such as one without a
        // writable `/tmp`, keeps its runner with an address of its own: a
        // program a job started then misses only the runner that starts next.
        let mut listener = match Listener::bind(Some(&address_name(installation))).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(event = "local_endpoint.unnamed", %error, "the local endpoint listens at an address of its own");
                Listener::bind(None).await?
            }
        };
        let endpoint = listener.endpoint().to_owned();
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let active = watch::Sender::new(0);
        let counted = active.clone();
        let owner = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            let mut backoff = demi_command_sdk::descriptors::Backoff::default();
            let mut outcome = loop {
                tokio::select! {
                    _ = stop.cancelled() => break Ok(()),
                    socket = listener.accept() => {
                        match socket {
                            Ok(socket) => {
                                backoff = demi_command_sdk::descriptors::Backoff::default();
                                let registration = ActiveConnection::new(&counted);
                                let handler = handler.clone();
                                let cancel = stop.child_token();
                                connections.spawn(async move {
                                    let _registration = registration;
                                    demi_command_sdk::serve_cancellable(socket, handler, cancel).await
                                });
                            }
                            // Out of open files, the connection stays queued until one
                            // closes (`crates/command-sdk/src/descriptors.rs`).
                            Err(error) if demi_command_sdk::descriptors::exhausted(&error) => backoff.wait().await,
                            Err(error) => break Err(error),
                        }
                    }
                    result = connections.join_next(), if !connections.is_empty() => {
                        match result {
                            Some(Ok(Err(ResidentError::CancellationDeadline))) => break Err(io::Error::other(ResidentError::CancellationDeadline)),
                            Some(Err(error)) => break Err(io::Error::other(error)),
                            // A failed client connection affects only its calls.
                            _ => {},
                        }
                    }
                }
            };
            stop.cancel();
            let closed = listener.close();
            while let Some(result) = connections.join_next().await {
                match result {
                    Ok(Err(ResidentError::CancellationDeadline)) => {
                        outcome = Err(io::Error::other(ResidentError::CancellationDeadline))
                    }
                    Err(error) => outcome = Err(io::Error::other(error)),
                    // Disconnect and cancellation errors have already reset the calls.
                    _ => {}
                }
            }
            outcome.and(closed)
        });
        Ok(Self {
            endpoint,
            cancel,
            owner: Some(owner),
            active,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn wait_idle(&self) {
        let mut active = self.active.subscribe();
        // The server holds the sender, so the wait ends only at zero.
        let _idle = active.wait_for(|count| *count == 0).await;
    }

    pub async fn close(mut self) -> io::Result<()> {
        self.cancel.cancel();
        self.owner
            .take()
            .expect("local server owner exists until close")
            .await
            .map_err(io::Error::other)?
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// The name of the endpoint address of the installation in `installation`:
/// the start of the SHA-256 of its path, short enough for a socket path, and
/// the same in every release.
fn address_name(installation: &std::path::Path) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(installation.to_string_lossy().as_bytes());
    digest[..8].iter().map(|byte| format!("{byte:02x}")).collect()
}
