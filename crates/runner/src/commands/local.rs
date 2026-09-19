//! Private local duplex endpoints used by shell builtins and external clients.

use demi_command_service::{Handler, ServiceError};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

struct Activity {
    count: AtomicUsize,
    changed: tokio::sync::Notify,
}
struct ActiveConnection(Arc<Activity>);
impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

pub struct Server {
    endpoint: String,
    cancel: CancellationToken,
    owner: Option<tokio::task::JoinHandle<io::Result<()>>>,
    activity: Arc<Activity>,
}

impl Server {
    pub async fn start(handler: Arc<dyn Handler>) -> io::Result<Self> {
        let mut listener = Listener::bind().await?;
        let endpoint = listener.endpoint().to_owned();
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let activity = Arc::new(Activity {
            count: AtomicUsize::new(0),
            changed: tokio::sync::Notify::new(),
        });
        let active = activity.clone();
        let owner = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            let mut backoff = demi_command_service::descriptors::Backoff::default();
            let mut outcome = loop {
                tokio::select! {
                    _ = stop.cancelled() => break Ok(()),
                    socket = listener.accept() => {
                        match socket {
                            Ok(socket) => {
                                backoff = demi_command_service::descriptors::Backoff::default();
                                active.count.fetch_add(1, Ordering::SeqCst);
                                let registration = ActiveConnection(active.clone());
                                let handler = handler.clone();
                                let cancel = stop.child_token();
                                connections.spawn(async move {
                                    let _registration = registration;
                                    demi_command_service::serve_cancellable(socket, handler, cancel).await
                                });
                            }
                            // Out of open files, the connection stays queued until one
                            // closes (`runner.md` § Load).
                            Err(error) if demi_command_service::descriptors::exhausted(&error) => backoff.wait().await,
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
            activity,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn wait_idle(&self) {
        loop {
            let changed = self.activity.changed.notified();
            if self.activity.count.load(Ordering::SeqCst) == 0 {
                return;
            }
            changed.await;
        }
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

pub trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}
pub type Stream = Box<dyn Duplex>;

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
            crate::fs::chmod(directory.path(), 0o700).await?;
            let path = directory.path().join("ipc.sock");
            let alive = std::fs::File::create(directory.path().join(ALIVE))?;
            alive.try_lock().map_err(io::Error::other)?;
            let socket = tokio::net::UnixListener::bind(&path)?;
            crate::fs::chmod(&path, 0o600).await?;
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

/// Waits for as long as the runner runs (`commands.md` § External command
/// clients); a runner that is gone fails at once.
pub async fn connect(endpoint: &str, cancel: &CancellationToken) -> io::Result<Stream> {
    tokio::select! {
        _ = cancel.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted, "local connection cancelled")),
        result = connect_inner(endpoint) => result,
    }
}

async fn connect_inner(endpoint: &str) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        if !std::path::Path::new(endpoint).is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "local socket path must be absolute",
            ));
        }
        let mut backoff = demi_command_service::descriptors::Backoff::default();
        loop {
            match tokio::net::UnixStream::connect(endpoint).await {
                Ok(socket) => return Ok(Box::new(socket)),
                // A full queue of waiting connections refuses the connection
                // (macOS) or defers it (Linux), and a runner that is gone
                // refuses it too. Out of open files here, the client waits as
                // well.
                Err(error)
                    if (matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionRefused | io::ErrorKind::WouldBlock
                    ) || demi_command_service::descriptors::exhausted(&error))
                        && runner_alive(endpoint) =>
                {
                    backoff.wait().await;
                }
                Err(error) => return Err(error),
            }
        }
    }
    #[cfg(windows)]
    {
        if !endpoint.starts_with(r"\\.\pipe\demi-") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid local named pipe endpoint",
            ));
        }
        loop {
            match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
                Ok(socket) => return Ok(Box::new(socket)),
                // ERROR_PIPE_BUSY means the server is preparing another instance.
                Err(error) if error.raw_os_error() == Some(231) => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

/// The file beside the socket that the runner keeps locked while it runs
/// (`commands.md` § External command clients).
#[cfg(unix)]
const ALIVE: &str = "ipc.alive";

/// A refused connection looks the same whether the runner is busy or gone;
/// only its lock tells them apart.
#[cfg(unix)]
fn runner_alive(endpoint: &str) -> bool {
    let path = std::path::Path::new(endpoint).with_file_name(ALIVE);
    match std::fs::File::open(path) {
        Ok(file) => matches!(file.try_lock_shared(), Err(std::fs::TryLockError::WouldBlock)),
        // Without an open file to read the lock with, assume the runner is
        // there; the client waits and looks again.
        Err(error) => demi_command_service::descriptors::exhausted(&error),
    }
}
