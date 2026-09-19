//! Resident shell job with bounded input/output and an owned completion task.

use super::{ShellOptions, scope::Scope};
use crate::process::{OutputChunk, OutputStream, ProcessExit, ProcessInput};
use bytes::Bytes;
use std::{
    collections::BTreeMap,
    fs::File,
    io,
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

pub struct Job {
    pub input: mpsc::Sender<ProcessInput>,
    pub output: mpsc::Receiver<OutputChunk>,
    exited: watch::Receiver<Option<(ProcessExit, Option<String>)>>,
    cancel: CancellationToken,
    requested_signal: Arc<OnceLock<String>>,
}

impl Job {
    pub fn start(
        script: String,
        cwd: PathBuf,
        mut env: BTreeMap<String, String>,
        live: bool,
        mut scope: Scope,
    ) -> io::Result<Self> {
        let (stdin, input_writer) = pipe()?;
        let (output_reader, stdout) = pipe()?;
        let (error_reader, stderr) = pipe()?;
        env.remove(crate::stdio::LIVE_INPUT_ENV);
        // Keep the reference handle alive until every interpreter task finishes.
        let input_reference = demi_command_service::descriptors::retry_blocking(|| stdin.try_clone())?;
        if live {
            env.insert(
                crate::stdio::LIVE_INPUT_ENV.into(),
                crate::stdio::live_reference(&input_reference)?,
            );
        }
        let (input, mut receiver) = mpsc::channel(4);
        let (sender, output) = mpsc::channel(4);
        let (finished, exited) = watch::channel(None);
        let cancel = scope.cancellation.clone();
        let owner_cancel = cancel.clone();
        let requested_signal = Arc::new(OnceLock::<String>::new());
        let owner_signal = requested_signal.clone();
        scope.cancellation = cancel.child_token();
        let input_scope = Scope::new(cancel.child_token(), None);
        let writer_scope = input_scope.clone();
        let writer = tokio::task::spawn_blocking(move || -> io::Result<()> {
            let runtime = tokio::runtime::Handle::current();
            loop {
                let item = runtime.block_on(async {
                    tokio::select! {
                        _ = writer_scope.cancellation.cancelled() => None,
                        item = receiver.recv() => item,
                    }
                });
                match item {
                    Some(ProcessInput::Bytes(mut bytes)) => {
                        while !bytes.is_empty() {
                            let count = writer_scope.write(&input_writer, &bytes)?;
                            if count == 0 {
                                return Err(io::ErrorKind::WriteZero.into());
                            }
                            bytes = bytes.slice(count..);
                        }
                    }
                    Some(ProcessInput::End) | None => return Ok(()),
                }
            }
        });
        let readers = [
            pump(
                output_reader,
                OutputStream::Stdout,
                sender.clone(),
                cancel.clone(),
            ),
            pump(error_reader, OutputStream::Stderr, sender, cancel.clone()),
        ];
        let worker = tokio::task::spawn_blocking(move || {
            let _input_reference = input_reference;
            let runtime = tokio::runtime::Handle::current();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.block_on(super::execute(
                    &script,
                    ShellOptions {
                        scope: scope.clone(),
                        login: true,
                        cwd,
                        env,
                        stdin,
                        stdout,
                        stderr,
                    },
                ))
            }));
            // Preserve successful process-substitution output until its producer
            // or consumer finishes. Failed execution cancels remaining work.
            if !matches!(outcome, Ok(Ok(_))) {
                scope.cancellation.cancel();
            }
            runtime.block_on(scope.finish());
            outcome
        });
        tokio::spawn(async move {
            let outcome = worker.await;
            input_scope.cancellation.cancel();
            let mut error = match writer.await {
                Ok(Err(error))
                    if !matches!(
                        error.kind(),
                        io::ErrorKind::BrokenPipe | io::ErrorKind::Interrupted
                    ) && !error
                        .get_ref()
                        .is_some_and(|error| error.is::<super::scope::Cancelled>())
                        && !owner_cancel.is_cancelled() =>
                {
                    Some(error.to_string())
                }
                Err(error) => Some(error.to_string()),
                _ => None,
            };
            input_scope.finish().await;
            for reader in readers {
                match reader.await {
                    Ok(Err(failure)) if !owner_cancel.is_cancelled() => {
                        error = Some(failure.to_string())
                    }
                    Err(failure) => error = Some(failure.to_string()),
                    _ => {}
                }
            }
            let (code, cwd) = match outcome {
                Ok(Ok(Ok(result))) => (
                    Some(i32::from(result.code)),
                    Some(result.cwd.to_string_lossy().into_owned()),
                ),
                Ok(Ok(Err(failure))) => {
                    error = Some(failure.to_string());
                    (None, None)
                }
                Ok(Err(_)) => {
                    error = Some("shell worker panicked".into());
                    (None, None)
                }
                Err(failure) => {
                    error = Some(failure.to_string());
                    (None, None)
                }
            };
            finished.send_replace(Some((
                ProcessExit {
                    code,
                    signal: owner_cancel.is_cancelled().then(|| {
                        owner_signal
                            .get()
                            .cloned()
                            .unwrap_or_else(|| "SIGKILL".into())
                    }),
                    error: if owner_cancel.is_cancelled() {
                        None
                    } else {
                        error
                    },
                },
                cwd,
            )));
        });
        Ok(Self {
            input,
            output,
            exited,
            cancel,
            requested_signal,
        })
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
    pub async fn signal(&self, signal: &str) -> io::Result<()> {
        match signal {
            "SIGINT" | "SIGTERM" | "SIGKILL" | "SIGHUP" | "SIGQUIT" => {
                if !self.cancel.is_cancelled() {
                    self.requested_signal.get_or_init(|| signal.into());
                }
                self.cancel();
                Ok(())
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported shell job signal",
            )),
        }
    }
    pub async fn wait(&mut self) -> (ProcessExit, Option<String>) {
        loop {
            if let Some(exit) = self.exited.borrow_and_update().clone() {
                return exit;
            }
            if self.exited.changed().await.is_err() {
                return (
                    ProcessExit {
                        code: None,
                        signal: None,
                        error: Some("shell owner ended without status".into()),
                    },
                    None,
                );
            }
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn pump(
    file: File,
    stream: OutputStream,
    sender: mpsc::Sender<OutputChunk>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<io::Result<()>> {
    tokio::task::spawn_blocking(move || {
        let scope = Scope::new(cancel.clone(), None);
        let runtime = tokio::runtime::Handle::current();
        let mut buffer = vec![0; 64 * 1024];
        let result = (|| {
            loop {
                let count = scope.read(&file, &mut buffer)?;
                if count == 0 {
                    return Ok(());
                }
                let chunk = OutputChunk {
                    stream,
                    bytes: Bytes::copy_from_slice(&buffer[..count]),
                };
                runtime.block_on(async {
                tokio::select! {
                    _ = cancel.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted, "job output cancelled")),
                    result = sender.send(chunk) => result.map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "job output consumer closed")),
                }
            })?;
            }
        })();
        runtime.block_on(scope.finish());
        result
    })
}

pub(super) fn pipe() -> io::Result<(File, File)> {
    // Out of open files, the job waits for one (`runner.md` § Load).
    let (reader, writer) = demi_command_service::descriptors::retry_blocking(std::io::pipe)?;
    #[cfg(unix)]
    {
        use std::os::fd::OwnedFd;
        Ok((
            File::from(OwnedFd::from(reader)),
            File::from(OwnedFd::from(writer)),
        ))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::OwnedHandle;
        Ok((
            File::from(OwnedHandle::from(reader)),
            File::from(OwnedHandle::from(writer)),
        ))
    }
}
