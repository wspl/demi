//! A job's declared commands in a shell process: each invocation goes to the
//! runner's local endpoint, as a command alias's does
//! (`crates/runner-process/src/command_client.rs`), and the runner's
//! dispatcher serves it.

use std::{future::Future, pin::Pin};

use bytes::Bytes;
use demi_command_protocol::{Completion, LocalInvocation, MediumFacts};
use demi_command_sdk::{
    Client, Exchange, ExchangeError, Handler, Input, InputSource, InvocationContext, Output,
    OutputSink, ResidentError,
};
use demi_runner_process::{command_client::RAW, local_socket};
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
            let socket = local_socket::connect_returning(&endpoint, &cancellation, local_socket::RUNNER_RETURN)
                .await
                .map_err(ResidentError::from)?;
            let (client, connection) = Client::connect(socket).await?;
            // The connection lives while the invocation does.
            let _driver = AbortOnDropHandle::new(tokio::spawn(connection));
            let (command_input, command_output) = client.invoke(&request).await?;
            let (mut source, mut sink) = (Source(input), Sink(output));
            let exchange = Exchange::new(command_input, command_output).run(&mut source, &mut sink);
            tokio::select! {
                _ = cancellation.cancelled() => Err(ResidentError::Cancelled),
                result = exchange => result.map_err(|error| match error {
                    ExchangeError::Resident(error)
                    | ExchangeError::Input(error)
                    | ExchangeError::Output(error) => error,
                }),
            }
        })
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
struct Sink(Output);

impl OutputSink for Sink {
    type Error = ResidentError;

    async fn stdout(&mut self, bytes: Bytes) -> Result<(), ResidentError> {
        self.0.stdout(bytes).await
    }

    async fn stderr(&mut self, bytes: Bytes) -> Result<(), ResidentError> {
        self.0.stderr(bytes).await
    }

    /// The runner hands a command's media to the job itself
    /// (`crates/runner-jobs/src/job_media.rs`), never to its caller.
    async fn medium(&mut self, _: MediumFacts, _: Bytes) -> Result<(), ResidentError> {
        Err(ResidentError::Failed("a local command returned a medium".into()))
    }
}
