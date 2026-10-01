//! The runner's private local endpoint (`commands.md` § External command
//! clients), which command aliases and the runner's management commands
//! reach.

use demi_command_protocol::LocalInvocation;
use demi_command_sdk::{Handler, ServiceError};
use demi_runner_process::command_client::Stream;
#[cfg(unix)]
use demi_runner_process::{command_client::ALIVE, private_files::chmod};
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
    pub async fn start(handler: Arc<dyn Handler<Metadata = LocalInvocation>>) -> io::Result<Self> {
        let mut listener = Listener::bind().await?;
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
                            // closes (`runner.md` § Load).
                            Err(error) if demi_command_sdk::descriptors::exhausted(&error) => backoff.wait().await,
                            Err(error) => break Err(error),
                        }
                    }
                    result = connections.join_next(), if !connections.is_empty() => {
                        match result {
                            Some(Ok(Err(ServiceError::CancellationDeadline))) => break Err(io::Error::other(ServiceError::CancellationDeadline)),
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
                    Ok(Err(ServiceError::CancellationDeadline)) => {
                        outcome = Err(io::Error::other(ServiceError::CancellationDeadline))
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

pub struct Listener {
    endpoint: String,
    #[cfg(unix)]
    socket: tokio::net::UnixListener,
    /// Locked exclusively while the runner runs; the system releases the lock
    /// when the runner exits, even by crashing.
    #[cfg(unix)]
    _alive: std::fs::File,
    #[cfg(unix)]
    directory: tempfile::TempDir,
    #[cfg(windows)]
    socket: tokio::net::windows::named_pipe::NamedPipeServer,
}

impl Listener {
    pub async fn bind() -> io::Result<Self> {
        #[cfg(unix)]
        {
            let directory = tempfile::Builder::new().prefix("demi-").tempdir()?;
            chmod(directory.path(), 0o700).await?;
            let path = directory.path().join("ipc.sock");
            let alive = std::fs::File::create(directory.path().join(ALIVE))?;
            alive.try_lock().map_err(io::Error::other)?;
            let socket = tokio::net::UnixListener::bind(&path)?;
            chmod(&path, 0o600).await?;
            let endpoint = path
                .to_str()
                .ok_or_else(|| io::Error::other("local endpoint path is not UTF-8"))?
                .to_owned();
            Ok(Self {
                endpoint,
                socket,
                _alive: alive,
                directory,
            })
        }
        #[cfg(windows)]
        {
            let endpoint = format!(r"\\.\pipe\demi-{}", uuid::Uuid::new_v4().simple());
            // The process token's default DACL controls local access. Remote
            // clients are rejected, and first-instance creation prevents taking
            // over an existing pipe. Invocation context secrets are checked by
            // the command dispatcher independently of transport access.
            let socket = tokio::net::windows::named_pipe::ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(&endpoint)?;
            Ok(Self { endpoint, socket })
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn accept(&mut self) -> io::Result<Stream> {
        #[cfg(unix)]
        {
            let (socket, _) = self.socket.accept().await?;
            Ok(Box::new(socket))
        }
        #[cfg(windows)]
        {
            self.socket.connect().await?;
            let next = tokio::net::windows::named_pipe::ServerOptions::new()
                .reject_remote_clients(true)
                .create(&self.endpoint)?;
            Ok(Box::new(std::mem::replace(&mut self.socket, next)))
        }
    }

    pub fn close(self) -> io::Result<()> {
        #[cfg(unix)]
        {
            drop(self.socket);
            self.directory.close()
        }
        #[cfg(windows)]
        {
            drop(self.socket);
            Ok(())
        }
    }
}
