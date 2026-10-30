//! A private local socket: a Unix socket in a directory only its user can
//! enter, or a named pipe only its account can open on Windows. The runner's
//! local endpoint (`crates/runner-jobs/src/commands/local.rs`) and each job's
//! shell (`crates/runner-shell/src/remote/mod.rs`) listen on one; the command
//! aliases and the runner connect to them.
//!
//! On Unix the listener keeps a file beside the socket locked while it
//! listens ([`ALIVE`]), so a client that is refused tells a busy listener,
//! which it waits for, from one that is gone, which fails it at once.

use std::io;
#[cfg(unix)]
use crate::private_files::chmod;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

/// Either end of a local endpoint's connection.
pub trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}
pub type Stream = Box<dyn Duplex>;

/// Waits for as long as the listener runs; one that is gone fails at once.
pub async fn connect(endpoint: &str, cancel: &CancellationToken) -> io::Result<Stream> {
    tokio::select! {
        _ = cancel.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted, "local connection cancelled")),
        result = connect_inner(endpoint) => result,
    }
}

/// How long a job's command waits for the runner's local endpoint to come
/// back: a runner that updates or crashes starts again within seconds, and
/// takes the job over (`crates/runner-shell/src/remote/mod.rs`).
pub const RUNNER_RETURN: std::time::Duration = std::time::Duration::from_secs(30);

/// Connects as [`connect`] does, and waits up to `patience` for a listener
/// that is gone to listen again, as the runner's endpoint does once the
/// runner has started again.
pub async fn connect_returning(
    endpoint: &str,
    cancel: &CancellationToken,
    patience: std::time::Duration,
) -> io::Result<Stream> {
    let until = tokio::time::Instant::now() + patience;
    loop {
        match connect(endpoint, cancel).await {
            Err(error) if gone(&error) && tokio::time::Instant::now() < until => {
                tokio::select! {
                    _ = cancel.cancelled() => return Err(io::Error::new(io::ErrorKind::Interrupted, "local connection cancelled")),
                    () = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
                }
            }
            result => return result,
        }
    }
}

/// Whether a connection failed because nothing listens at its address.
fn gone(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
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
        let mut backoff = demi_command_sdk::descriptors::Backoff::default();
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
                    ) || demi_command_sdk::descriptors::exhausted(&error))
                        && liveness(endpoint) != Liveness::Gone =>
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

/// The file beside the socket that the runner keeps locked while it runs.
///
/// A refused connection looks the same whether the runner is busy or gone. A
/// client that is refused therefore checks the lock: it waits while the lock is
/// held, and fails at once otherwise. The system releases the lock when the
/// runner exits, even by crashing.
#[cfg(unix)]
pub const ALIVE: &str = "ipc.alive";

/// What the runner's lock says about it.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Liveness {
    /// It holds its lock: it runs, and may be too busy to accept.
    Running,
    /// Nothing holds the lock, or there is none: it stopped or crashed.
    Gone,
    /// The lock cannot be read without an open file; the client waits and
    /// looks again (`crates/command-sdk/src/descriptors.rs`).
    Unknown,
}

/// A refused connection looks the same whether the runner is busy or gone;
/// only its lock tells them apart.
#[cfg(unix)]
fn liveness(endpoint: &str) -> Liveness {
    let path = std::path::Path::new(endpoint).with_file_name(ALIVE);
    match std::fs::File::open(path) {
        Ok(file) => match file.try_lock_shared() {
            Err(std::fs::TryLockError::WouldBlock) => Liveness::Running,
            // An unlocked file, or one the system cannot lock, is a crashed
            // runner's leftover.
            Ok(()) | Err(std::fs::TryLockError::Error(_)) => Liveness::Gone,
        },
        Err(error) if demi_command_sdk::descriptors::exhausted(&error) => Liveness::Unknown,
        Err(_) => Liveness::Gone,
    }
}

pub struct Listener {
    endpoint: String,
    #[cfg(unix)]
    socket: tokio::net::UnixListener,
    /// Locked exclusively while the listener runs; the system releases the
    /// lock when its process exits, even by crashing.
    #[cfg(unix)]
    _alive: std::fs::File,
    #[cfg(unix)]
    directory: Directory,
    #[cfg(windows)]
    socket: tokio::net::windows::named_pipe::NamedPipeServer,
    /// Who may connect to each instance of the pipe, as SDDL.
    #[cfg(windows)]
    access: Vec<u16>,
}

/// Where a listener's socket lives on Unix.
#[cfg(unix)]
enum Directory {
    /// A directory of its own, which goes with it.
    Own(tempfile::TempDir),
    /// The directory its name gives, which stays for the next listener of
    /// that name.
    Named(std::path::PathBuf),
}

impl Listener {
    /// Listens at a new address of its own, or at the one `name` gives,
    /// which a later listener of that name takes over once this one is gone:
    /// the runner's local endpoint has one address per installation, so a
    /// program a job started before the runner started again reaches the
    /// new one (`crates/runner-shell/src/remote/mod.rs`).
    ///
    /// On Unix a named address is a socket in `/tmp/demi-<uid>/<name>/`, as
    /// tmux keeps its sockets in `/tmp/tmux-<uid>/`: a directory that its
    /// user alone can enter, which the listener refuses when another user
    /// owns it or may enter it. On Windows it is the pipe `demi-<name>`.
    pub async fn bind(name: Option<&str>) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let directory = match name {
                None => {
                    let directory = tempfile::Builder::new().prefix("demi-").tempdir()?;
                    chmod(directory.path(), 0o700).await?;
                    Directory::Own(directory)
                }
                Some(name) => Directory::Named(named_directory(name)?),
            };
            let root = match &directory {
                Directory::Own(directory) => directory.path(),
                Directory::Named(path) => path.as_path(),
            };
            let path = root.join("ipc.sock");
            let alive = std::fs::File::create(root.join(ALIVE))?;
            alive.try_lock().map_err(|error| match error {
                std::fs::TryLockError::WouldBlock => io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("another process listens at {}", path.display()),
                ),
                std::fs::TryLockError::Error(error) => error,
            })?;
            // A listener that ended without its cleanup left its socket.
            match std::fs::remove_file(&path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
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
            let endpoint = match name {
                None => format!(r"\\.\pipe\demi-{}", uuid::Uuid::new_v4().simple()),
                Some(name) => format!(r"\\.\pipe\demi-{name}"),
            };
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
            match self.directory {
                Directory::Own(directory) => directory.close(),
                // The directory stays for the next listener of its name.
                Directory::Named(directory) => match std::fs::remove_file(directory.join("ipc.sock")) {
                    Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                    _ => Ok(()),
                },
            }
        }
        #[cfg(windows)]
        {
            drop(self.socket);
            Ok(())
        }
    }
}

/// The directory of the named address `name`: `/tmp/demi-<uid>/<name>`, each
/// level made for its user alone, and refused when another user owns it or
/// may enter it.
#[cfg(unix)]
fn named_directory(name: &str) -> io::Result<std::path::PathBuf> {
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
    if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "an address name of letters, digits and dashes"));
    }
    let user = rustix::process::getuid().as_raw();
    let base = std::path::Path::new("/tmp").join(format!("demi-{user}"));
    let mut path = base.clone();
    for level in [None, Some(name)] {
        if let Some(level) = level {
            path = base.join(level);
        }
        match std::fs::DirBuilder::new().mode(0o700).create(&path) {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
            _ => {}
        }
        // Not followed: a link put there by another user leads elsewhere.
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.uid() != user || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{} is not a directory its user alone can enter", path.display()),
            ));
        }
    }
    Ok(path)
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
