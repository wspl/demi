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
    sync::mpsc,
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
    pub kind: SpawnErrorKind,
    pub message: String,
}

pub use crate::connection::wire::{OutputStream, Signal};
use crate::connection::wire::SpawnErrorKind;

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
    signals: mpsc::Sender<Signal>,
    /// Ends with the process's exit, once it is reaped and its pipes drained.
    owner: Option<tokio::task::JoinHandle<ProcessExit>>,
    exited: Option<ProcessExit>,
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
        let (signals, mut signal_rx) = mpsc::channel::<Signal>(4);
        let cancel = CancellationToken::new();
        let owner_cancel = cancel.clone();
        let owner = tokio::spawn(async move {
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
                        if let Some(signal) = signal && let Err(error) = send_signal(child.as_mut(), signal) {
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
            match status {
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
            }
        });
        Ok(Self {
            input,
            output,
            signals,
            owner: Some(owner),
            exited: None,
            cancel,
            pid,
        })
    }

    pub async fn signal(&self, signal: Signal) -> io::Result<()> {
        self.signals
            .send(signal)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "process has exited"))
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// The process's exit, once it is reaped and its pipes drained.
    pub async fn wait(&mut self) -> ProcessExit {
        if let Some(owner) = self.owner.take() {
            self.exited = Some(owner.await.unwrap_or_else(|_| ProcessExit {
                code: None,
                signal: None,
                error: Some("the process's owner panicked".into()),
            }));
        }
        self.exited.clone().expect("the owner left the exit")
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
        Err(error) if error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error()) => Ok(()),
        result => result,
    }
}

fn send_signal(child: &mut dyn ChildWrapper, signal: Signal) -> io::Result<()> {
    #[cfg(unix)]
    {
        match child.signal(number(signal).as_raw()) {
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error()) => {
                Ok(())
            }
            result => result,
        }
    }
    #[cfg(windows)]
    {
        match signal {
            Signal::Terminate | Signal::Kill | Signal::Interrupt => kill(child),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported Windows process signal",
            )),
        }
    }
}

#[cfg(unix)]
fn number(signal: Signal) -> rustix::process::Signal {
    use rustix::process::Signal as Number;
    match signal {
        Signal::Terminate => Number::TERM,
        Signal::Kill => Number::KILL,
        Signal::Interrupt => Number::INT,
        Signal::Hangup => Number::HUP,
        Signal::Quit => Number::QUIT,
        Signal::User1 => Number::USR1,
        Signal::User2 => Number::USR2,
        Signal::Stop => Number::STOP,
        Signal::Continue => Number::CONT,
    }
}

/// The name of the signal that ended a process: one of the set requests
/// name, `SIGPIPE`, or its number.
fn exit_signal(status: ExitStatus) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        const SIGNALS: [Signal; 9] = [
            Signal::Terminate,
            Signal::Kill,
            Signal::Interrupt,
            Signal::Hangup,
            Signal::Quit,
            Signal::User1,
            Signal::User2,
            Signal::Stop,
            Signal::Continue,
        ];
        status.signal().map(|raw| {
            match SIGNALS.into_iter().find(|signal| number(*signal).as_raw() == raw) {
                Some(signal) => signal.to_string(),
                None if raw == rustix::process::Signal::PIPE.as_raw() => "SIGPIPE".into(),
                None => format!("SIG{raw}"),
            }
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
        SpawnErrorKind::CwdUnusable
    } else {
        match error.kind() {
            io::ErrorKind::NotFound => SpawnErrorKind::ExecutableNotFound,
            io::ErrorKind::PermissionDenied => SpawnErrorKind::PermissionDenied,
            io::ErrorKind::IsADirectory => SpawnErrorKind::IsDirectory,
            _ => SpawnErrorKind::Other,
        }
    };
    SpawnFailure {
        kind,
        message: error.to_string(),
    }
}
