//! One invocation driven to its completion (`native-runtime.md` § Invocation
//! protocol): each input pull of the service is answered with one chunk from
//! the caller's source, and the service's records are sorted into standard
//! output, standard error and the one completion.
//!
//! For example, a command client reads its terminal only when the service
//! asks for input, and writes what comes back to its own standard output and
//! error, while a user stream reads the page's pipe and uploads to another.
//! Both are an [`Exchange`] with their own [`InputSource`] and [`OutputSink`].
//! A medium the service returns reaches the sink whole.

use std::future::Future;

use bytes::{Bytes, BytesMut};
use demi_command_protocol::{Completion, ProtocolError, Record};
use tokio::sync::mpsc;

use crate::{CommandInput, CommandOutput, ServiceError};

/// Where an invocation's input comes from.
pub trait InputSource {
    type Error;

    /// The next chunk, at most a record long, or `None` at the end.
    fn next(&mut self) -> impl Future<Output = Result<Option<Bytes>, Self::Error>>;
}

/// Where an invocation's output goes.
pub trait OutputSink {
    type Error;

    fn stdout(&mut self, bytes: Bytes) -> impl Future<Output = Result<(), Self::Error>>;

    fn stderr(&mut self, bytes: Bytes) -> impl Future<Output = Result<(), Self::Error>>;

    /// A medium the service returned (`commands.md` § Return media), whole.
    fn medium(&mut self, bytes: Bytes) -> impl Future<Output = Result<(), Self::Error>>;
}

/// Which side of an exchange failed.
#[derive(Debug)]
pub enum ExchangeError<I, O> {
    /// The service, or the connection to it, failed or broke the protocol.
    Service(ServiceError),
    Input(I),
    Output(O),
}

/// An invocation's input and output, from [`crate::Client::invoke`].
pub struct Exchange {
    input: CommandInput,
    output: CommandOutput,
}

impl Exchange {
    pub fn new(input: CommandInput, output: CommandOutput) -> Self {
        Self { input, output }
    }

    /// Runs the invocation until its completion. The end of the source ends
    /// the input only; the invocation runs on until it completes. Any failure
    /// cancels the invocation.
    pub async fn run<S: InputSource, K: OutputSink>(
        self,
        source: &mut S,
        sink: &mut K,
    ) -> Result<Completion, ExchangeError<S::Error, K::Error>> {
        let Self {
            mut input,
            mut output,
        } = self;
        let (pull, mut demanded) = mpsc::channel::<()>(1);
        let send = async {
            while demanded.recv().await.is_some() {
                match source.next().await.map_err(ExchangeError::Input)? {
                    Some(bytes) => input.write(bytes).await.map_err(ExchangeError::Service)?,
                    None => {
                        input.end().map_err(ExchangeError::Service)?;
                        break;
                    }
                }
            }
            // The completion, not the input, ends the exchange.
            std::future::pending().await
        };
        let receive = async {
            let mut completion = None;
            // The medium whose bytes are arriving: its size, and its bytes so far.
            let mut medium: Option<(u64, BytesMut)> = None;
            while let Some(record) = output.next().await.map_err(ExchangeError::Service)? {
                if medium.is_some() && !matches!(record, Record::MediumBytes(_)) {
                    return Err(broken("a medium's bytes are interleaved with another record"));
                }
                match record {
                    Record::Medium { size } => {
                        medium = Some((size, BytesMut::new()));
                    }
                    Record::MediumBytes(bytes) => {
                        let Some((size, received)) = medium.as_mut() else {
                            return Err(broken("medium bytes without a medium"));
                        };
                        if received.len() as u64 + bytes.len() as u64 > *size {
                            return Err(broken("a medium's bytes exceed its size"));
                        }
                        received.extend_from_slice(&bytes);
                    }
                    Record::Stdout(bytes) => {
                        sink.stdout(bytes).await.map_err(ExchangeError::Output)?
                    }
                    Record::Stderr(bytes) => {
                        sink.stderr(bytes).await.map_err(ExchangeError::Output)?
                    }
                    Record::InputPull => pull.try_send(()).map_err(|_| {
                        ExchangeError::Service(
                            ProtocolError::Invalid("overlapping input demands".into()).into(),
                        )
                    })?,
                    Record::Completion(value) => completion = Some(value),
                }
                if let Some((size, received)) = &mut medium
                    && received.len() as u64 == *size
                {
                    let bytes = std::mem::take(received).freeze();
                    medium = None;
                    sink.medium(bytes).await.map_err(ExchangeError::Output)?;
                }
            }
            completion.ok_or(ExchangeError::Service(ProtocolError::Incomplete.into()))
        };
        let result = tokio::select! {
            result = receive => result,
            result = send => result,
        };
        if result.is_err() {
            input.cancel();
        }
        result
    }
}

/// A service that broke the record protocol.
fn broken<I, O>(reason: &str) -> ExchangeError<I, O> {
    ExchangeError::Service(ProtocolError::Invalid(reason.into()).into())
}

/// The input of a local invocation, for a caller that forwards it.
impl InputSource for crate::Input {
    type Error = ServiceError;

    async fn next(&mut self) -> Result<Option<Bytes>, ServiceError> {
        crate::Input::next(self).await
    }
}
