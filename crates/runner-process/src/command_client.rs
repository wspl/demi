//! The local command client: a
//! command alias forwards its command line to the runner's local endpoint and
//! streams its standard input and output through it.
//!
//! A command outlives its runner, which an update replaces. A call to the
//! backend that waits for what outlives the runner too, such as a vendor's
//! media job, comes back: the runner tells the client the call's id
//! ([`demi_command_protocol::Record::Resumable`]), and a client that loses
//! the runner before the end sends the command line again to the next one,
//! naming the call to resume ([`Resuming`]), unless it wrote some of its
//! standard output, which the call would write again.

use bytes::Bytes;
use demi_command_protocol::{Completion, LocalInvocation, MAX_RECORD_BYTES};
use demi_command_sdk::{
    Client, CommandInput, CommandOutput, Exchange, ExchangeError, InputSource, OutputSink,
};
use serde::{Deserialize, Serialize};
use std::{io, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::local_socket::connect_returning;
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

pub const ENDPOINT_ENV: &str = "DEMI_RUNNER_ENDPOINT";
pub const CONTEXT_ENV: &str = "DEMI_CONTEXT_ID";

/// The local operation that runs a command line, whose arguments are a
/// [`RawCommand`].
pub const RAW: &str = "raw";

/// A command line a client forwards to the runner: the execution context it
/// runs in, its root command and arguments, and whether it has an input. A
/// request that breaks these rules is refused as it is read.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, try_from = "Request")]
pub struct RawCommand {
    pub context: String,
    pub root: String,
    pub argv: Vec<String>,
    /// Whether the command's stdin is something to read: false for the null
    /// device ([`crate::stdio::is_null_device`]).
    pub stdin: bool,
    /// Whether the client resumes the command's call on the next runner,
    /// and so is told the call it waits for.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resumable: bool,
    /// The backend's call the command resumes, which a runner before this
    /// one sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
}

/// A request as it arrives, before its check.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    context: String,
    root: String,
    argv: Vec<String>,
    #[serde(default)]
    stdin: Option<bool>,
    /// Whether the command's stdin is its job's input that ends only with
    /// the job, which a job's shell of an earlier release, one that outlived
    /// its runner, sends instead of `stdin`: such an input is none to read.
    #[serde(default)]
    live: Option<bool>,
    #[serde(default)]
    resumable: bool,
    #[serde(default)]
    resume: Option<String>,
}

impl TryFrom<Request> for RawCommand {
    type Error = io::Error;

    fn try_from(request: Request) -> io::Result<Self> {
        let stdin = request.stdin.unwrap_or(request.live != Some(true));
        let mut raw = RawCommand::new(request.context, request.root, request.argv, stdin)?;
        raw.resumable = request.resumable;
        raw.resume = request.resume;
        Ok(raw)
    }
}

/// What a client that resumes its command's call knows of it: the call the
/// runner said it waits for, and whether some of its standard output was
/// written.
#[derive(Debug, Default)]
pub struct Resuming {
    call: Option<String>,
    wrote: bool,
}

impl Resuming {
    /// `invocation`, a raw command line, asking the runner to tell it the
    /// call it waits for; any other invocation as it is.
    pub fn asking(invocation: &LocalInvocation) -> io::Result<LocalInvocation> {
        Self::with(invocation, |raw| raw.resumable = true)
    }

    pub fn told(&mut self, call: String) {
        self.call = Some(call);
    }

    pub fn wrote(&mut self) {
        self.wrote = true;
    }

    /// What to send the next runner once this one went away before the
    /// end: `invocation` resuming the call it waits for, if the runner said
    /// it does and none of its standard output was written. The next runner
    /// says it again before a later resume.
    pub fn again(&mut self, invocation: &LocalInvocation) -> Option<LocalInvocation> {
        let call = self.call.take().filter(|_| !self.wrote)?;
        Self::with(invocation, |raw| raw.resume = Some(call)).ok()
    }

    fn with(invocation: &LocalInvocation, change: impl FnOnce(&mut RawCommand)) -> io::Result<LocalInvocation> {
        if invocation.operation != RAW {
            return Ok(invocation.clone());
        }
        let mut raw: RawCommand = serde_json::from_value(invocation.args.clone()).map_err(io::Error::other)?;
        change(&mut raw);
        Ok(LocalInvocation {
            args: serde_json::to_value(raw).map_err(io::Error::other)?,
            ..invocation.clone()
        })
    }
}

impl RawCommand {
    /// A request whose context is a context id, whose root is a single
    /// command name, and whose arguments hold no NUL.
    pub fn new(context: String, root: String, argv: Vec<String>, stdin: bool) -> io::Result<Self> {
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
            stdin,
            resumable: false,
            resume: None,
        })
    }
}

pub struct Stdio<I, O, E> {
    pub stdin: I,
    pub stdout: O,
    pub stderr: E,
}

/// Input is read only after an explicit pull. Output and connection processing
/// continue while that read waits, including when nothing is ever written.
/// A runner that is gone is waited for up to `patience`, as a job's command
/// waits for one that starts again ([`crate::local_socket::RUNNER_RETURN`]),
/// and a call that outlives it goes on with the next ([`Resuming`]).
pub async fn forward<I, O, E>(
    endpoint: &str,
    request: &LocalInvocation,
    stdio: Stdio<I, O, E>,
    cancel: CancellationToken,
    patience: Duration,
) -> io::Result<Completion>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    E: AsyncWrite + Unpin,
{
    let mut sink = Terminal {
        stdout: stdio.stdout,
        stderr: stdio.stderr,
        resuming: Resuming::default(),
    };
    let mut source = Reader(stdio.stdin);
    let mut request = Resuming::asking(request)?;
    loop {
        let sent = forward_once(endpoint, &request, &mut source, &mut sink, &cancel, patience).await;
        match sent {
            // The runner went away: a failure of the caller's own input or
            // output ends the command.
            Err(Failure::Runner(error)) if !cancel.is_cancelled() => match sink.resuming.again(&request) {
                Some(again) => {
                    tracing::info!(event = "command.resumed", %error, "a command's call goes on with the next runner");
                    request = again;
                }
                None => return Err(error),
            },
            Err(Failure::Runner(error) | Failure::Caller(error)) => return Err(error),
            Ok(completion) => return Ok(completion),
        }
    }
}

/// What ended an invocation before its completion.
enum Failure {
    /// The runner, or the connection to it.
    Runner(io::Error),
    /// The caller's input or output, or its cancellation.
    Caller(io::Error),
}

/// Sends `request` to the runner that listens now, as [`forward`] does.
async fn forward_once<I, O, E>(
    endpoint: &str,
    request: &LocalInvocation,
    source: &mut Reader<I>,
    sink: &mut Terminal<O, E>,
    cancel: &CancellationToken,
    patience: Duration,
) -> Result<Completion, Failure>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    E: AsyncWrite + Unpin,
{
    let socket = connect_returning(endpoint, cancel, patience).await.map_err(Failure::Runner)?;
    let (client, connection) = Client::connect(socket).await.map_err(|error| Failure::Runner(io::Error::other(error)))?;
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
            _ = cancel.cancelled() => return Err(Failure::Caller(cancelled())),
            result = tokio::time::timeout(Duration::from_secs(10), client.invoke(request)) => result
                .map_err(|error| Failure::Runner(io::Error::other(error)))?
                .map_err(|error| Failure::Runner(io::Error::other(error)))?,
        };
        exchange(input, output, source, sink, cancel).await
    };
    tokio::pin!(result);
    let outcome = tokio::select! {
        biased;
        result = &mut result => result,
        result = &mut driver => return Err(Failure::Runner(match result {
            Ok(Err(error)) => error,
            Err(error) => io::Error::other(error),
            Ok(Ok(())) => io::Error::new(io::ErrorKind::UnexpectedEof, "local connection ended before command completion"),
        })),
    };
    stop.cancel();
    driver.await.map_err(|error| Failure::Runner(io::Error::other(error)))?.map_err(Failure::Runner)?;
    outcome
}

/// Runs the invocation with the caller's standard input as its source and
/// the caller's standard output and error as its sink.
async fn exchange<I, O, E>(
    input: CommandInput,
    output: CommandOutput,
    source: &mut Reader<I>,
    sink: &mut Terminal<O, E>,
    cancel: &CancellationToken,
) -> Result<Completion, Failure>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    E: AsyncWrite + Unpin,
{
    let exchange = Exchange::new(input, output).run(source, sink);
    // Dropping a cancelled exchange resets its invocation.
    let completion = tokio::select! {
        _ = cancel.cancelled() => return Err(Failure::Caller(cancelled())),
        result = exchange => result.map_err(|error| match error {
            ExchangeError::Resident(error) => Failure::Runner(io::Error::other(error)),
            ExchangeError::Input(error) | ExchangeError::Output(error) => Failure::Caller(error),
        })?,
    };
    sink.stdout.flush().await.map_err(Failure::Caller)?;
    sink.stderr.flush().await.map_err(Failure::Caller)?;
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
    resuming: Resuming,
}

impl<O: AsyncWrite + Unpin, E: AsyncWrite + Unpin> OutputSink for Terminal<O, E> {
    type Error = io::Error;

    async fn stdout(&mut self, bytes: Bytes) -> io::Result<()> {
        self.resuming.wrote();
        self.stdout.write_all(&bytes).await
    }

    async fn resumable(&mut self, call: String) -> io::Result<()> {
        self.resuming.told(call);
        Ok(())
    }

    async fn stderr(&mut self, bytes: Bytes) -> io::Result<()> {
        self.stderr.write_all(&bytes).await
    }

    /// The runner hands the media of the commands it runs to their job; it
    /// sends a local caller only their output.
    async fn medium(&mut self, _: demi_command_protocol::MediumFacts, _: Bytes) -> io::Result<()> {
        Err(io::Error::other("a local command returned a medium"))
    }
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "command cancelled")
}

