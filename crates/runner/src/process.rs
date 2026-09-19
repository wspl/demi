use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    process::{ExitStatus, Stdio},
};

use bytes::Bytes;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{mpsc, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct SpawnOptions {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub process_group: bool,
}

#[derive(Debug, Clone)]
pub struct SpawnFailure {
    pub kind: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

pub struct OutputChunk {
    pub stream: OutputStream,
    pub bytes: Bytes,
}

#[derive(Debug, Clone)]
pub struct ProcessExit {
    pub code: Option<i32>,
    pub signal: Option<String>,
    pub error: Option<String>,
}

pub enum ProcessInput {
    Bytes(Bytes),
    End,
}

/// Handles never own child processes implicitly: their owner task reaps on every
/// completion path, and dropping a handle cancels that owner.
pub struct ChildProcess {
    pub input: mpsc::Sender<ProcessInput>,
    pub output: mpsc::Receiver<OutputChunk>,
    signals: mpsc::Sender<String>,
    exited: watch::Receiver<Option<ProcessExit>>,
    cancel: CancellationToken,
    pub pid: u32,
}

impl ChildProcess {
    pub async fn spawn(options: SpawnOptions) -> Result<Self, SpawnFailure> {
        let mut command = Command::new(&options.command);
        command
            .args(&options.args)
            .current_dir(&options.cwd)
            .env_clear()
            .envs(&options.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut command = CommandWrap::from(command);
        command.wrap(KillOnDrop);
        if options.process_group {
            #[cfg(unix)]
            command.wrap(process_wrap::tokio::ProcessGroup::leader());
            #[cfg(windows)]
            command.wrap(process_wrap::tokio::JobObject);
        }
        // Out of open files, the process waits for one (`runner.md` § Load).
        let mut backoff = demi_command_service::descriptors::Backoff::default();
        let mut child = loop {
            match command.spawn() {
                Ok(child) => break child,
                Err(error) if demi_command_service::descriptors::exhausted(&error) => {
                    backoff.wait().await
                }
                Err(error) => return Err(classify_failure(error, &options).await),
            }
        };
        let pid = child.id().expect("newly spawned process has an ID");
        let mut stdin = child.stdin().take().expect("piped stdin");
        let stdout = child.stdout().take().expect("piped stdout");
        let stderr = child.stderr().take().expect("piped stderr");
        let (input, mut input_rx) = mpsc::channel(4);
        let (output_tx, output) = mpsc::channel(4);
        let (signals, mut signal_rx) = mpsc::channel::<String>(4);
        let (exit_tx, exited) = watch::channel(None);
        let cancel = CancellationToken::new();
        let owner_cancel = cancel.clone();
        tokio::spawn(async move {
            let io_cancel = CancellationToken::new();
            let stdin_cancel = CancellationToken::new();
            let mut readers = JoinSet::new();
            readers.spawn(pump(
                stdout,
                OutputStream::Stdout,
                output_tx.clone(),
                io_cancel.clone(),
            ));
            readers.spawn(pump(
                stderr,
                OutputStream::Stderr,
                output_tx,
                io_cancel.clone(),
            ));
            let writer_cancel = stdin_cancel.clone();
            let writer = tokio::spawn(async move {
                tokio::select! {
                    _ = writer_cancel.cancelled() => Ok(()),
                    result = async {
                        while let Some(input) = input_rx.recv().await {
                            match input {
                                ProcessInput::Bytes(bytes) => stdin.write_all(&bytes).await?,
                                ProcessInput::End => break,
                            }
                        }
                        stdin.shutdown().await
                    } => result,
                }
            });
            let mut failure = None;
            let status = loop {
                tokio::select! {
                    biased;
                    _ = owner_cancel.cancelled() => {
                        if let Err(error) = kill(child.as_mut()) {
                            failure = Some(error.to_string());
                        }
                        break child.wait().await;
                    }
                    signal = signal_rx.recv(), if !signal_rx.is_closed() => {
                        if let Some(signal) = signal && let Err(error) = send_signal(child.as_mut(), &signal) {
                            failure = Some(error.to_string());
                        }
                    }
                    status = child.wait() => break status,
                    reader = readers.join_next(), if !readers.is_empty() => {
                        match reader {
                            Some(Ok(Err(error))) => {
                                failure = Some(error.to_string());
                                owner_cancel.cancel();
                            }
                            Some(Err(error)) => {
                                failure = Some(error.to_string());
                                owner_cancel.cancel();
                            }
                            _ => {},
                        }
                    }
                }
            };
            stdin_cancel.cancel();
            match writer.await {
                Ok(Err(error)) if error.kind() != io::ErrorKind::BrokenPipe => {
                    failure = Some(error.to_string());
                }
                Err(error) => {
                    failure = Some(error.to_string());
                }
                // The child may close stdin before finishing; this is not a process failure.
                _ => {}
            }
            if options.process_group {
                // The leader's exit also ends this invocation's descendants.
                if let Err(error) = kill(child.as_mut()) {
                    failure = Some(error.to_string());
                }
            }
            while !readers.is_empty() {
                tokio::select! {
                    biased;
                    _ = owner_cancel.cancelled(), if !io_cancel.is_cancelled() => {
                        // The leader may have exited while a descendant still owns its pipes.
                        if let Err(error) = kill(child.as_mut()) {
                            failure = Some(error.to_string());
                        }
                        io_cancel.cancel();
                    }
                    reader = readers.join_next() => match reader {
                        Some(Ok(Err(error))) => { failure = Some(error.to_string()); }
                        Some(Err(error)) => { failure = Some(error.to_string()); }
                        _ => {},
                    }
                }
            }
            let result = match status {
                Ok(status) => ProcessExit {
                    code: status.code(),
                    signal: exit_signal(status),
                    error: failure,
                },
                Err(error) => ProcessExit {
                    code: None,
                    signal: None,
                    error: Some(error.to_string()),
                },
            };
            exit_tx.send_replace(Some(result));
        });
        Ok(Self {
            input,
            output,
            signals,
            exited,
            cancel,
            pid,
        })
    }

    pub async fn signal(&self, signal: &str) -> io::Result<()> {
        self.signals
            .send(signal.into())
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "process has exited"))
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub async fn wait(&mut self) -> ProcessExit {
        loop {
            if let Some(exit) = self.exited.borrow_and_update().clone() {
                return exit;
            }
            if self.exited.changed().await.is_err() {
                return ProcessExit {
                    code: None,
                    signal: None,
                    error: Some("process owner ended without status".into()),
                };
            }
        }
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn pump<T: AsyncRead + Unpin>(
    mut input: T,
    stream: OutputStream,
    output: mpsc::Sender<OutputChunk>,
    cancel: CancellationToken,
) -> io::Result<()> {
    tokio::select! {
        _ = cancel.cancelled() => Ok(()),
        result = async {
            let mut buffer = vec![0; 64 * 1024];
            loop {
                let count = input.read(&mut buffer).await?;
                if count == 0 { return Ok(()); }
                if output.send(OutputChunk { stream, bytes: Bytes::copy_from_slice(&buffer[..count]) }).await.is_err() {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "process output consumer closed"));
                }
            }
        } => result,
    }
}

pub(crate) fn kill(child: &mut dyn ChildWrapper) -> io::Result<()> {
    match child.start_kill() {
        #[cfg(unix)]
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
        result => result,
    }
}

fn send_signal(child: &mut dyn ChildWrapper, signal: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        let number = match signal {
            "SIGTERM" => libc::SIGTERM,
            "SIGKILL" => libc::SIGKILL,
            "SIGINT" => libc::SIGINT,
            "SIGHUP" => libc::SIGHUP,
            "SIGQUIT" => libc::SIGQUIT,
            "SIGUSR1" => libc::SIGUSR1,
            "SIGUSR2" => libc::SIGUSR2,
            "SIGSTOP" => libc::SIGSTOP,
            "SIGCONT" => libc::SIGCONT,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported process signal",
                ));
            }
        };
        match child.signal(number) {
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
            result => result,
        }
    }
    #[cfg(windows)]
    {
        match signal {
            "SIGTERM" | "SIGKILL" | "SIGINT" => kill(child),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported Windows process signal",
            )),
        }
    }
}

fn exit_signal(status: ExitStatus) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal().map(|number| match number {
            libc::SIGTERM => "SIGTERM".into(),
            libc::SIGKILL => "SIGKILL".into(),
            libc::SIGINT => "SIGINT".into(),
            libc::SIGHUP => "SIGHUP".into(),
            libc::SIGQUIT => "SIGQUIT".into(),
            libc::SIGPIPE => "SIGPIPE".into(),
            _ => format!("SIG{number}"),
        })
    }
    #[cfg(windows)]
    {
        let _ = status;
        None
    }
}

async fn classify_failure(error: io::Error, options: &SpawnOptions) -> SpawnFailure {
    let kind = if !tokio::fs::metadata(&options.cwd)
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        "cwd_unusable"
    } else {
        match error.kind() {
            io::ErrorKind::NotFound => "executable_not_found",
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::IsADirectory => "is_directory",
            _ => "other",
        }
    };
    SpawnFailure {
        kind,
        message: error.to_string(),
    }
}
