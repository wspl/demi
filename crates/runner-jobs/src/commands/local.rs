//! The runner's private local endpoint (`crates/runner-process/src/command_client.rs`), which command aliases and the runner's management commands
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
                            // closes (`crates/command-sdk/src/descriptors.rs`).
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
    /// Who may connect to each instance of the pipe, as SDDL.
    #[cfg(windows)]
    access: Vec<u16>,
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
            // Only the account the runner runs as may connect, from any of its
            // sessions: an elevated one, such as an SSH logon of an
            // administrator, and a desktop one alike.
            //
            // Remote clients are rejected, and first-instance creation prevents
            // taking over an existing pipe. Invocation context secrets are
            // checked by the command dispatcher independently of transport
            // access.
            let access = pipe_access::account_only()?;
            let socket = pipe_access::create(
                tokio::net::windows::named_pipe::ServerOptions::new()
                    .first_pipe_instance(true)
                    .reject_remote_clients(true),
                &endpoint,
                &access,
            )?;
            Ok(Self {
                endpoint,
                socket,
                access,
            })
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
            let next = pipe_access::create(
                tokio::net::windows::named_pipe::ServerOptions::new().reject_remote_clients(true),
                &self.endpoint,
                &self.access,
            )?;
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

/// Who may open the local endpoint's pipe on Windows: the account the runner
/// runs as, whatever session it opens it from, and no one else.
///
/// Without it the pipe took the default access of the runner's token. For an
/// administrator's elevated session, such as an SSH logon, that grants the
/// Administrators group rather than the account. `run.ps1 uninstall` from a
/// desktop window, which holds the group only to deny, then failed with "Access
/// is denied. (os error 5)" on a runner started over SSH.
#[cfg(windows)]
mod pipe_access {
    use std::io;
    use std::ptr::null_mut;

    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// The pipe's security descriptor as NUL-terminated SDDL: full access
    /// for the runner's account alone, protected from inheritance, and a
    /// medium integrity label, so that the account's ordinary processes may
    /// write to a pipe an elevated runner made, and sandboxed ones may not.
    pub(super) fn account_only() -> io::Result<Vec<u16>> {
        let sid = account_sid()?;
        Ok(format!("D:P(A;;GA;;;{sid})S:(ML;;NW;;;ME)")
            .encode_utf16()
            .chain([0])
            .collect())
    }

    /// Creates an instance of the pipe `endpoint` with `options`, which
    /// `access` alone may open.
    pub(super) fn create(
        options: &ServerOptions,
        endpoint: &str,
        access: &[u16],
    ) -> io::Result<NamedPipeServer> {
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `access` is NUL-terminated UTF-16; the call allocates the
        // descriptor, which is freed below.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                access.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        // SAFETY: `attributes` is a valid SECURITY_ATTRIBUTES whose
        // descriptor lives until after the call.
        let created =
            unsafe { options.create_with_security_attributes_raw(endpoint, (&raw mut attributes).cast()) };
        // SAFETY: the conversion allocated the descriptor with LocalAlloc,
        // and the pipe keeps a copy of its own. A failed free leaks a few
        // bytes, which is safe to ignore.
        unsafe { LocalFree(descriptor) };
        created
    }

    /// The SID of the account the runner runs as, in its string form.
    fn account_sid() -> io::Result<String> {
        let mut token: HANDLE = null_mut();
        // SAFETY: the pseudo handle of the current process needs no closing;
        // the token handle is closed below.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let sid = token_user_sid(token);
        // SAFETY: `token` is the open handle from above. A failed close
        // leaks one handle, which is safe to ignore.
        unsafe { CloseHandle(token) };
        sid
    }

    fn token_user_sid(token: HANDLE) -> io::Result<String> {
        let mut length = 0u32;
        // SAFETY: a null buffer of length 0 asks only for the length; the
        // call fails with ERROR_INSUFFICIENT_BUFFER, which is expected.
        unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut length) };
        // u64 words keep TOKEN_USER, which holds pointers, aligned.
        let mut buffer = vec![0u64; (length as usize).div_ceil(size_of::<u64>())];
        // SAFETY: `buffer` holds at least `length` bytes.
        let read = unsafe {
            GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), length, &mut length)
        };
        if read == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call wrote a TOKEN_USER at the start of `buffer`,
        // whose SID points into `buffer`, which outlives its use here.
        let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        let mut text = null_mut();
        // SAFETY: the SID is valid; the call allocates `text`, freed below.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `text` is a NUL-terminated UTF-16 string the call made.
        let sid = unsafe {
            let length = (0..).take_while(|&index| *text.add(index) != 0).count();
            String::from_utf16_lossy(std::slice::from_raw_parts(text, length))
        };
        // SAFETY: allocated with LocalAlloc by the conversion. A failed
        // free leaks a few bytes, which is safe to ignore.
        unsafe { LocalFree(text.cast()) };
        Ok(sid)
    }
}
