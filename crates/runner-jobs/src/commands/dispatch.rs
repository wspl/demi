//! The local API validates declarations before routing native and callback work.

use crate::{
    commands::artifacts::JobArtifacts,
    commands::command_output::{CommandOutput, Media},
    commands::contexts::Contexts,
    commands::rpc,
};
use bytes::Bytes;
use demi_command_declarations::UsageError;
use demi_command_protocol::{Completion, Invocation, LocalInvocation};
use demi_command_sdk::{
    Exchange, ExchangeError, Handler, Input, InvocationContext, Output, ServiceError,
};
use demi_runner_command_packages::ServiceHandle;
use demi_runner_process::{
    command_client::{RAW, RawCommand},
    pipes::PipeClient,
};
use futures_util::FutureExt;
use std::{future::Future, pin::Pin, sync::Arc};

/// The program's name, which a failure that names no command begins with.
pub const RUNNER: &str = "demi-runner";

/// The most a command's body read from its standard input may hold.
const BODY_BYTES: usize = 1024 * 1024;

/// Runs the declared commands of live contexts, for the local endpoint and
/// for declared builtins alike.
#[derive(Clone)]
pub struct Dispatcher {
    pub contexts: Contexts,
    pub services: ServiceHandle,
    pub pipes: PipeClient,
}

impl Handler for Dispatcher {
    type Metadata = LocalInvocation;

    fn operations(&self) -> Vec<String> {
        vec![RAW.into()]
    }

    fn invoke(
        &self,
        context: InvocationContext<LocalInvocation>,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let dispatcher = self.clone();
        Box::pin(async move {
            let output = context.output.clone();
            match context.request.operation.as_str() {
                RAW => dispatcher.command(context).await,
                operation => reported(
                    Err(ServiceError::UnknownOperation(operation.into())),
                    &output,
                    RUNNER,
                )
                .await,
            }
        })
    }
}

/// A local invocation's end as its caller sees it: a failure other than a
/// cancellation is told on its standard error as a runtime error of the
/// program or command `name`, `<name>: <error>`, and ends it with 1
/// (`commands.md` § Handle an rpc call).
pub async fn reported(
    result: Result<Completion, ServiceError>,
    output: &Output,
    name: &str,
) -> Result<Completion, ServiceError> {
    match result {
        Err(ServiceError::Cancelled) => Err(ServiceError::Cancelled),
        Err(error) => {
            output.stderr(format!("{name}: {error}\n").into()).await?;
            Ok(completed(1))
        }
        Ok(completion) => Ok(completion),
    }
}

/// Why a command line did not run to its handler's end.
enum Failure {
    /// It does not fit its command: clap's text, told whole, exit 2.
    Usage(UsageError),
    /// What stopped it, told after the command's path, exit 1.
    Service(ServiceError),
}

impl From<ServiceError> for Failure {
    fn from(error: ServiceError) -> Self {
        Self::Service(error)
    }
}

impl From<UsageError> for Failure {
    fn from(error: UsageError) -> Self {
        Self::Usage(error)
    }
}

impl Dispatcher {
    /// Runs a command line: a usage error ends it with 2 and clap's text, a
    /// runtime error with 1 and a line that begins with the command's path,
    /// as far as the line names a command.
    async fn command(
        &self,
        invocation: InvocationContext<LocalInvocation>,
    ) -> Result<Completion, ServiceError> {
        let output = invocation.output.clone();
        let raw: RawCommand = match serde_json::from_value(invocation.request.args.clone()) {
            Ok(raw) => raw,
            Err(error) => return reported(Err(error.into()), &output, RUNNER).await,
        };
        let mut name = raw.root.clone();
        let result = self.run(raw, invocation, &mut name).await;
        match result {
            Ok(completion) => Ok(completion),
            Err(Failure::Usage(error)) => {
                output.stderr(format!("{error}\n").into()).await?;
                Ok(completed(2))
            }
            Err(Failure::Service(error)) => reported(Err(error), &output, &name).await,
        }
    }

    /// Runs `raw`; `name` becomes the command's path once the command line
    /// selects one.
    async fn run(
        &self,
        raw: RawCommand,
        mut invocation: InvocationContext<LocalInvocation>,
        name: &mut String,
    ) -> Result<Completion, Failure> {
        let context = self
            .contexts
            .get(&raw.context)
            .map_err(ServiceError::failed)?;
        let root = context
            .manifest
            .roots
            .get(&raw.root)
            .ok_or_else(|| ServiceError::failed(DispatchError::NotARoot))?;
        let selected = root.tree.select(&raw.argv)?;
        *name = selected.path.join(" ");
        let parsed = selected.parse(&raw.argv)?;
        let path = name.clone();
        let execute = async {
            if parsed.help {
                invocation
                    .output
                    .stdout(format!("{}\n", selected.node.help(&path)).into())
                    .await?;
                return Ok(completed(0));
            }
            let leaf = selected
                .node
                .leaf()
                .ok_or_else(|| ServiceError::failed(DispatchError::NotALeaf))?;
            let body = if leaf.stdin_target(&parsed.values).is_some() {
                // A terminal is the live channel, not a finite command body.
                let mut bytes = Vec::new();
                if !raw.live {
                    while let Some(chunk) = invocation.input.next().await? {
                        if bytes.len() + chunk.len() > BODY_BYTES {
                            return Err(ServiceError::failed(DispatchError::BodyTooLarge).into());
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                }
                Some(
                    String::from_utf8(bytes)
                        .map_err(|_| ServiceError::failed(DispatchError::BodyNotText))?,
                )
            } else {
                None
            };
            let parsed = parsed.validate(leaf, body)?;
            let mut output = CommandOutput::new(
                invocation.output,
                leaf.json_output().filter(|_| parsed.json),
                Media::new(leaf.media, raw.stdout, context.media.clone()),
            );
            let _hint = rpc::running_hint(
                &context.connection,
                &context.job_id,
                leaf.running_hint.as_deref(),
            )
            .await?;
            let code = if let Some(binding) = leaf.binding() {
                let descriptor = context
                    .manifest
                    .packages
                    .get(&binding.descriptor_hash)
                    .ok_or_else(|| ServiceError::failed(DispatchError::MissingDescriptor))?;
                let resolver = Arc::new(JobArtifacts::new(self.contexts.clone()));
                // What the program asks for during the call is located for
                // this job (`native-runtime.md` § The artifacts stream).
                let _invoking = self.services.invoking(
                    &invocation.request.invocation_id,
                    &descriptor.id,
                    resolver.clone(),
                );
                let mut resident = self
                    .services
                    .acquire(
                        descriptor,
                        resolver,
                        Arc::new(context.connection.clone()),
                        &invocation.cancellation,
                    )
                    .await
                    .map_err(ServiceError::failed)?;
                let request = Invocation {
                    context: context.command.clone(),
                    json: Some(parsed.json),
                    edits: Some(context.edits.clone()),
                    stdout: Some(raw.stdout),
                    operation: binding.operation.clone(),
                    invocation_id: invocation.request.invocation_id,
                    command: path.clone(),
                    args: parsed.values.into(),
                    cwd: invocation.request.cwd,
                    env: invocation.request.env,
                };
                let exchange = async {
                    let (input, response) = resident
                        .client()
                        .invoke(&request)
                        .await
                        .map_err(Failed::Service)?;
                    native_exchange(input, response, &mut invocation.input, &mut output, &path)
                        .await
                };
                match exchange.await {
                    Ok(code) => code,
                    Err(
                        Failed::Caller(error) | Failed::Service(error @ ServiceError::Cancelled),
                    ) => {
                        return Err(error.into());
                    }
                    // A call that failed with its service reports how the
                    // service ended (`native-runtime.md` § Invocation protocol).
                    Err(Failed::Service(error)) => {
                        return Err(ServiceError::failed(resident.failure(error).await).into());
                    }
                }
            } else {
                let mut argv = vec![raw.root.clone()];
                argv.extend(raw.argv.iter().cloned());
                // Boxed as a `dyn` future: the call's future nests deep
                // enough that checking it is `Send` within this one would
                // exceed the compiler's recursion limit.
                FutureExt::boxed(rpc::invoke(
                    &self.pipes,
                    rpc::Request {
                        context: context.clone(),
                        root: raw.root.clone(),
                        argv,
                        parsed,
                        cwd: invocation.request.cwd,
                        env: invocation.request.env,
                        live: raw.live,
                        finite: !raw.live && leaf.stdin_field.is_none(),
                    },
                    invocation.input,
                    &mut output,
                    invocation.cancellation.clone(),
                ))
                .await?
            };
            output.finish(code).await?;
            Ok(completed(code))
        };
        tokio::select! {
            biased;
            _ = context.cancel.cancelled() => Err(ServiceError::Cancelled.into()),
            _ = invocation.cancellation.cancelled() => Err(ServiceError::Cancelled.into()),
            result = execute => result,
        }
    }
}

/// Which end of a native call failed: the service or the caller.
enum Failed {
    Service(ServiceError),
    Caller(ServiceError),
}

/// Drives one native call: the caller's input goes to the service one chunk
/// per pull, and the service's records come back to the caller. A failure
/// the service reports is told after the command's `path`, its code left
/// to `--json` callers of the package.
async fn native_exchange(
    sender: demi_command_sdk::CommandInput,
    response: demi_command_sdk::CommandOutput,
    input: &mut Input,
    output: &mut CommandOutput<'_>,
    path: &str,
) -> Result<u8, Failed> {
    let completion = Exchange::new(sender, response)
        .run(input, output)
        .await
        .map_err(|error| match error {
            ExchangeError::Service(error) => Failed::Service(error),
            ExchangeError::Input(error) | ExchangeError::Output(error) => Failed::Caller(error),
        })?;
    if let Some(error) = completion.error {
        output
            .stderr(Bytes::from(format!("{path}: {}\n", error.message)))
            .await
            .map_err(Failed::Caller)?;
    }
    Ok(completion.exit_code)
}

/// A completion with `exit_code` and no error.
pub fn completed(exit_code: u8) -> Completion {
    Completion {
        exit_code,
        error: None,
    }
}
/// Why the runner could not run a command a job asked for.
#[derive(Debug, thiserror::Error)]
enum DispatchError {
    #[error("not a root command of this manifest")]
    NotARoot,
    #[error("missing command leaf")]
    NotALeaf,
    #[error("stdin: the body exceeds 1 MiB")]
    BodyTooLarge,
    #[error("stdin: the body is not UTF-8 text")]
    BodyNotText,
    #[error("native descriptor is not in this manifest")]
    MissingDescriptor,
}
