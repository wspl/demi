//! The local API validates declarations before routing native and callback work.

use crate::{
    commands::artifacts::JobArtifacts,
    commands::command_client::RawCommand,
    commands::command_output::CommandOutput,
    commands::contexts::Contexts,
    commands::rpc,
    management::{self, Management},
    pipes::PipeClient,
    services::ServiceHandle,
};
use bytes::Bytes;
use demi_command_service::protocol::{Completion, Invocation, LocalInvocation};
use demi_command_service::{
    Exchange, ExchangeError, Handler, Input, InvocationContext, ServiceError,
};
use std::{future::Future, pin::Pin, sync::Arc};

/// The most a command's body read from its standard input may hold.
const BODY_BYTES: usize = 1024 * 1024;

/// Runs the declared commands of live contexts, for the local endpoint and
/// for declared builtins alike.
#[derive(Clone)]
pub struct Dispatcher {
    pub contexts: Contexts,
    pub services: ServiceHandle,
    pub pipes: PipeClient,
    pub management: Arc<Management>,
}

impl Handler for Dispatcher {
    type Metadata = LocalInvocation;

    fn operations(&self) -> Vec<String> {
        vec!["raw".into(), "manage".into()]
    }

    fn invoke(
        &self,
        context: InvocationContext<LocalInvocation>,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let dispatcher = self.clone();
        Box::pin(async move {
            let output = context.output.clone();
            let result = match context.request.operation.as_str() {
                "raw" => dispatcher.command(context).await,
                "manage" => dispatcher.manage(context).await,
                operation => Err(ServiceError::UnknownOperation(operation.into())),
            };
            match result {
                Err(ServiceError::Cancelled) => Err(ServiceError::Cancelled),
                Err(error) => {
                    output
                        .stderr(format!("demi-runner: {error}\n").into())
                        .await?;
                    Ok(completed(1))
                }
                Ok(completion) => Ok(completion),
            }
        })
    }
}

impl Dispatcher {
    async fn manage(
        &self,
        context: InvocationContext<LocalInvocation>,
    ) -> Result<Completion, ServiceError> {
        let request: management::Request = serde_json::from_value(context.request.args)?;
        if !self.management.authorize(&request) {
            return Err(ServiceError::failed(DispatchError::Secret));
        }
        if matches!(request.action, management::Action::Drain) {
            self.management.draining.cancel();
        }
        context
            .output
            .stdout(serde_json::to_vec(&self.management.status())?.into())
            .await?;
        Ok(completed(0))
    }

    async fn command(
        &self,
        mut invocation: InvocationContext<LocalInvocation>,
    ) -> Result<Completion, ServiceError> {
        let raw: RawCommand = serde_json::from_value(invocation.request.args.clone())?;
        let context = self.contexts.get(&raw.context).map_err(ServiceError::failed)?;
        let execute = async {
            let root = context.manifest.roots.get(&raw.root).ok_or_else(|| {
                ServiceError::failed(DispatchError::NotARoot(raw.root.clone()))
            })?;
            let selected = root.tree.select(&raw.argv).map_err(ServiceError::failed)?;
            let parsed = selected.parse(&raw.argv).map_err(ServiceError::failed)?;
            if parsed.help {
                let path = selected.path.join(" ");
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
            let body = if leaf.stdin_field.is_some() {
                // A terminal is the live channel, not a finite command body.
                let mut bytes = Vec::new();
                if !raw.live {
                    while let Some(chunk) = invocation.input.next().await? {
                        if bytes.len() + chunk.len() > BODY_BYTES {
                            return Err(ServiceError::failed(DispatchError::BodyTooLarge));
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                }
                Some(String::from_utf8(bytes).map_err(ServiceError::failed)?)
            } else {
                None
            };
            let parsed = parsed.validate(leaf, body).map_err(ServiceError::failed)?;
            let mut output =
                CommandOutput::new(invocation.output, leaf.json_output().filter(|_| parsed.json));
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
                let mut resident = self
                    .services
                    .acquire(descriptor, resolver, &invocation.cancellation)
                    .await
                    .map_err(ServiceError::failed)?;
                let request = Invocation {
                    context: context.command.clone(),
                    json: Some(parsed.json),
                    edits: Some(context.edits.clone()),
                    operation: binding.operation.clone(),
                    invocation_id: invocation.request.invocation_id,
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
                    native_exchange(input, response, &mut invocation.input, &mut output).await
                };
                match exchange.await {
                    Ok(code) => code,
                    Err(Failed::Caller(error) | Failed::Service(error @ ServiceError::Cancelled)) => {
                        return Err(error);
                    }
                    // A call that failed with its service reports how the
                    // service ended (`native-runtime.md` § Invocation protocol).
                    Err(Failed::Service(error)) => {
                        return Err(ServiceError::failed(resident.failure(error).await));
                    }
                }
            } else {
                let mut argv = vec![raw.root.clone()];
                argv.extend(raw.argv);
                rpc::invoke(
                    &self.pipes,
                    rpc::Request {
                        context: context.clone(),
                        root: raw.root,
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
                )
                .await?
            };
            output.finish(code).await?;
            Ok(completed(code))
        };
        tokio::select! {
            biased;
            _ = context.cancel.cancelled() => Err(ServiceError::Cancelled),
            _ = invocation.cancellation.cancelled() => Err(ServiceError::Cancelled),
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
/// per pull, and the service's records come back to the caller.
async fn native_exchange(
    sender: demi_command_service::CommandInput,
    response: demi_command_service::CommandOutput,
    input: &mut Input,
    output: &mut CommandOutput<'_>,
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
            .stderr(Bytes::from(format!("{}: {}\n", error.code, error.message)))
            .await
            .map_err(Failed::Caller)?;
    }
    Ok(completion.exit_code)
}

fn completed(exit_code: u8) -> Completion {
    Completion {
        exit_code,
        error: None,
    }
}
/// Why the runner could not run a command a job asked for.
#[derive(Debug, thiserror::Error)]
enum DispatchError {
    #[error("invalid management secret")]
    Secret,
    #[error("{0}: not a root command of this manifest")]
    NotARoot(String),
    #[error("missing command leaf")]
    NotALeaf,
    #[error("command body exceeds 1 MiB")]
    BodyTooLarge,
    #[error("native descriptor is not in this manifest")]
    MissingDescriptor,
}
