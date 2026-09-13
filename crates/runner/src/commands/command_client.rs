//! Streaming local command forwarding, shared by embedded and external callers.

use bytes::Bytes;
use demi_command_service::{
    Client, CommandInput, CommandOutput,
    protocol::{Completion, Invocation, MAX_RECORD_BYTES, Record},
};
use serde::{Deserialize, Serialize};
use std::{io, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc,
};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

pub const ENDPOINT_ENV: &str = "DEMI_RUNNER_ENDPOINT";
pub const CONTEXT_ENV: &str = "DEMI_CONTEXT_ID";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawCommand {
    pub context: String,
    pub root: String,
    pub argv: Vec<String>,
    pub live: bool,
}

impl RawCommand {
    pub fn validate(&self) -> io::Result<()> {
        if self.context.len() != 32
            || !self.context.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.root.is_empty()
            || self.root.contains(['/', '\\', '\0'])
            || self.argv.iter().any(|arg| arg.contains('\0'))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid local command request",
            ));
        }
        Ok(())
    }
}

pub struct Stdio<I, O, E> {
    pub stdin: I,
    pub stdout: O,
    pub stderr: E,
}

/// Input is read only after an explicit pull. Output and connection processing
/// continue while that read waits, including when the user never types anything.
pub async fn forward<I, O, E>(
    endpoint: &str,
    request: &Invocation,
    stdio: Stdio<I, O, E>,
    cancel: CancellationToken,
) -> io::Result<Completion>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    E: AsyncWrite + Unpin,
{
    let socket = crate::commands::local::connect(endpoint, &cancel).await?;
    let (client, connection) = Client::connect(socket).await.map_err(io::Error::other)?;
    let stop = CancellationToken::new();
    let _stop_guard = stop.clone().drop_guard();
    let stopped = stop.clone();
    let mut driver = AbortOnDropHandle::new(tokio::spawn(async move {
        tokio::select! {
            result = connection => result.map_err(io::Error::other),
            _ = stopped.cancelled() => Ok(()),
        }
    }));
    let result = async {
        let (input, output) = tokio::select! {
            _ = cancel.cancelled() => return Err(cancelled()),
            result = tokio::time::timeout(Duration::from_secs(10), client.invoke(request)) => result.map_err(io::Error::other)?.map_err(io::Error::other)?,
        };
        exchange(input, output, stdio, &cancel).await
    };
    tokio::pin!(result);
    let outcome = tokio::select! {
        biased;
        result = &mut result => result,
        result = &mut driver => return match result {
            Ok(Err(error)) => Err(error),
            Err(error) => Err(io::Error::other(error)),
            Ok(Ok(())) => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "local connection ended before command completion")),
        },
    };
    stop.cancel();
    driver.await.map_err(io::Error::other)??;
    outcome
}

async fn exchange<I, O, E>(
    mut input: CommandInput,
    mut output: CommandOutput,
    mut stdio: Stdio<I, O, E>,
    cancel: &CancellationToken,
) -> io::Result<Completion>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    E: AsyncWrite + Unpin,
{
    let (pull, mut demanded) = mpsc::channel::<()>(1);
    let send = async {
        let mut buffer = vec![0; MAX_RECORD_BYTES];
        while demanded.recv().await.is_some() {
            let count = stdio.stdin.read(&mut buffer).await?;
            if count == 0 {
                input.end().map_err(io::Error::other)?;
                // EOF ends input only. Keep waiting for the command's completion.
                return std::future::pending::<io::Result<Completion>>().await;
            }
            input
                .write(Bytes::copy_from_slice(&buffer[..count]))
                .await
                .map_err(io::Error::other)?;
        }
        std::future::pending::<io::Result<Completion>>().await
    };
    let receive = async {
        let mut completion = None;
        while let Some(record) = output.next().await.map_err(io::Error::other)? {
            match record {
                Record::Stdout(bytes) => stdio.stdout.write_all(&bytes).await?,
                Record::Stderr(bytes) => stdio.stderr.write_all(&bytes).await?,
                Record::InputPull => pull.try_send(()).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "overlapping stdin demands")
                })?,
                Record::Completion(value) => completion = Some(value),
            }
        }
        stdio.stdout.flush().await?;
        stdio.stderr.flush().await?;
        completion.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "missing command completion")
        })
    };
    let result = tokio::select! {
        _ = cancel.cancelled() => Err(cancelled()),
        result = receive => result,
        result = send => result,
    };
    if result.is_err() {
        input.cancel();
    }
    result
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "command cancelled")
}
