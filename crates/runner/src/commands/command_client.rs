//! Streaming local command forwarding, shared by embedded and external callers.

use bytes::Bytes;
use demi_command_service::{
    Client, CommandInput, CommandOutput, Exchange, ExchangeError, InputSource, OutputSink,
    protocol::{Completion, LocalInvocation, MAX_RECORD_BYTES},
};
use serde::{Deserialize, Serialize};
use std::{io, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

pub const ENDPOINT_ENV: &str = "DEMI_RUNNER_ENDPOINT";
pub const CONTEXT_ENV: &str = "DEMI_CONTEXT_ID";

/// A command line a client forwards to the runner (`commands.md` § External
/// command clients): the execution context it runs in, its root command and
/// arguments, and whether its input is the job's live terminal. A request
/// that breaks these rules is refused as it is read.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, try_from = "Request")]
pub struct RawCommand {
    pub context: String,
    pub root: String,
    pub argv: Vec<String>,
    pub live: bool,
}

/// A request as it arrives, before its check.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    context: String,
    root: String,
    argv: Vec<String>,
    live: bool,
}

impl TryFrom<Request> for RawCommand {
    type Error = io::Error;

    fn try_from(request: Request) -> io::Result<Self> {
        RawCommand::new(request.context, request.root, request.argv, request.live)
    }
}

impl RawCommand {
    /// A request whose context is a context id, whose root is a single
    /// command name, and whose arguments hold no NUL.
    pub fn new(context: String, root: String, argv: Vec<String>, live: bool) -> io::Result<Self> {
        if context.len() != 32
            || !context.bytes().all(|byte| byte.is_ascii_hexdigit())
            || root.is_empty()
            || root.contains(['/', '\\', '\0'])
            || argv.iter().any(|arg| arg.contains('\0'))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid local command request",
            ));
        }
        Ok(Self {
            context,
            root,
            argv,
            live,
        })
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
    request: &LocalInvocation,
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

/// Runs the invocation with the caller's standard input as its source and
/// the caller's standard output and error as its sink.
async fn exchange<I, O, E>(
    input: CommandInput,
    output: CommandOutput,
    stdio: Stdio<I, O, E>,
    cancel: &CancellationToken,
) -> io::Result<Completion>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    E: AsyncWrite + Unpin,
{
    let mut source = Reader(stdio.stdin);
    let mut sink = Terminal {
        stdout: stdio.stdout,
        stderr: stdio.stderr,
    };
    let exchange = Exchange::new(input, output).run(&mut source, &mut sink);
    // Dropping a cancelled exchange resets its invocation.
    let completion = tokio::select! {
        _ = cancel.cancelled() => return Err(cancelled()),
        result = exchange => result.map_err(|error| match error {
            ExchangeError::Service(error) => io::Error::other(error),
            ExchangeError::Input(error) | ExchangeError::Output(error) => error,
        })?,
    };
    sink.stdout.flush().await?;
    sink.stderr.flush().await?;
    Ok(completion)
}

/// A reader that yields a record-sized chunk per pull.
struct Reader<R>(R);

impl<R: AsyncRead + Unpin> InputSource for Reader<R> {
    type Error = io::Error;

    async fn next(&mut self) -> io::Result<Option<Bytes>> {
        let mut buffer = vec![0; MAX_RECORD_BYTES];
        let count = self.0.read(&mut buffer).await?;
        buffer.truncate(count);
        Ok((count > 0).then(|| Bytes::from(buffer)))
    }
}

/// The caller's standard output and error.
struct Terminal<O, E> {
    stdout: O,
    stderr: E,
}

impl<O: AsyncWrite + Unpin, E: AsyncWrite + Unpin> OutputSink for Terminal<O, E> {
    type Error = io::Error;

    async fn stdout(&mut self, bytes: Bytes) -> io::Result<()> {
        self.stdout.write_all(&bytes).await
    }

    async fn stderr(&mut self, bytes: Bytes) -> io::Result<()> {
        self.stderr.write_all(&bytes).await
    }
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "command cancelled")
}
