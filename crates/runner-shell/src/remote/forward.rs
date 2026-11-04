//! A job's declared commands in a shell process: each invocation goes to the
//! runner's local endpoint, as a command alias's does
//! (`crates/runner-process/src/command_client.rs`), and the runner's
//! dispatcher serves it. A call that outlives the runner goes on with the
//! next one, as an alias's does.

use std::{future::Future, pin::Pin};

use bytes::Bytes;
use demi_command_protocol::{Completion, LocalInvocation, MediumFacts};
use demi_command_sdk::{
    Client, Exchange, ExchangeError, Handler, Input, InputSource, InvocationContext, Output,
    OutputSink, ResidentError,
};
use demi_runner_process::{
    command_client::{RAW, Resuming},
    local_socket,
};
use tokio_util::task::AbortOnDropHandle;

/// Forwards each invocation to the runner's local endpoint.
pub(super) struct Forward {
    pub endpoint: String,
}

impl Handler for Forward {
    type Metadata = LocalInvocation;

    fn operations(&self) -> Vec<String> {
        vec![RAW.into()]
    }

    fn invoke(
        &self,
        context: InvocationContext<LocalInvocation>,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ResidentError>> + Send>> {
        let endpoint = self.endpoint.clone();
        Box::pin(async move {
            let InvocationContext {
                request,
                input,
                output,
                cancellation,
            } = context;
            let (mut source, mut sink) = (Source(input), Sink { output, resuming: Resuming::default() });
            let mut request = Resuming::asking(&request).map_err(ResidentError::from)?;
            loop {
                let sent = forward_once(&endpoint, &request, &mut source, &mut sink, &cancellation).await;
                match sent {
                    // The runner went away: a failure of the job's own
                    // input or output ends the command.
                    Err(Failure::Runner(error)) if !cancellation.is_cancelled() => {
                        match sink.resuming.again(&request) {
                            Some(again) => {
                                tracing::info!(event = "command.resumed", %error, "a command's call goes on with the next runner");
                                request = again;
                            }
                            None => return Err(error),
                        }
                    }
                    Err(Failure::Runner(error) | Failure::Job(error)) => return Err(error),
                    Ok(completion) => return Ok(completion),
                }
            }
        })
    }
}

/// What ended an invocation before its completion.
enum Failure {
    /// The runner, or the connection to it.
    Runner(ResidentError),
    /// The job's input or output.
    Job(ResidentError),
}

/// Sends `request` to the runner that listens now.
async fn forward_once(
    endpoint: &str,
    request: &LocalInvocation,
    source: &mut Source,
    sink: &mut Sink,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<Completion, Failure> {
    let socket = local_socket::connect_returning(endpoint, cancellation, local_socket::RUNNER_RETURN)
        .await
        .map_err(|error| Failure::Runner(error.into()))?;
    let (client, connection) = Client::connect(socket).await.map_err(Failure::Runner)?;
    // The connection lives while the invocation does.
    let _driver = AbortOnDropHandle::new(tokio::spawn(connection));
    let (command_input, command_output) = client.invoke(request).await.map_err(Failure::Runner)?;
    let exchange = Exchange::new(command_input, command_output).run(source, sink);
    tokio::select! {
        _ = cancellation.cancelled() => Err(Failure::Job(ResidentError::Cancelled)),
        result = exchange => result.map_err(|error| match error {
            ExchangeError::Resident(error) => Failure::Runner(error),
            ExchangeError::Input(error) | ExchangeError::Output(error) => Failure::Job(error),
        }),
    }
}

/// The builtin's standard input, which the job's shell reads.
struct Source(Input);

impl InputSource for Source {
    type Error = ResidentError;

    async fn next(&mut self) -> Result<Option<Bytes>, ResidentError> {
        self.0.next().await
    }
}

/// The builtin's standard output and error.
struct Sink {
    output: Output,
    resuming: Resuming,
}

impl OutputSink for Sink {
    type Error = ResidentError;

    async fn stdout(&mut self, bytes: Bytes) -> Result<(), ResidentError> {
        self.resuming.wrote();
        self.output.stdout(bytes).await
    }

    async fn stderr(&mut self, bytes: Bytes) -> Result<(), ResidentError> {
        self.output.stderr(bytes).await
    }

    async fn resumable(&mut self, call: String) -> Result<(), ResidentError> {
        self.resuming.told(call);
        Ok(())
    }

    /// The runner hands a command's media to the job itself
    /// (`crates/runner-jobs/src/job_media.rs`), never to its caller.
    async fn medium(&mut self, _: MediumFacts, _: Bytes) -> Result<(), ResidentError> {
        Err(ResidentError::Failed("a local command returned a medium".into()))
    }
}
