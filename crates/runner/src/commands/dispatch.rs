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
                _ => Err(handler("unknown runner operation")),
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
            return Err(handler("invalid management secret"));
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
        raw.validate().map_err(handler)?;
        let context = self.contexts.get(&raw.context).map_err(handler)?;
        let execute = async {
            let root = context.manifest.roots.get(&raw.root).ok_or_else(|| {
                handler(format!("{}: not a root command of this manifest", raw.root))
            })?;
            let selected = root.tree.select(&raw.argv).map_err(handler)?;
            let parsed = selected.parse(&raw.argv).map_err(handler)?;
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
                .ok_or_else(|| handler("missing command leaf"))?;
            let body = if leaf.stdin_field.is_some() {
                // A terminal is the live channel, not a finite command body.
                let mut bytes = Vec::new();
                if !raw.live {
                    while let Some(chunk) = invocation.input.next().await? {
                        if bytes.len() + chunk.len() > 1024 * 1024 {
                            return Err(handler("command body exceeds 1 MiB"));
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                }
                Some(String::from_utf8(bytes).map_err(handler)?)
            } else {
                None
            };
            let parsed = parsed.validate(leaf, body).map_err(handler)?;
            let mut output = CommandOutput::new(invocation.output, parsed.json);
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
                    .ok_or_else(|| handler("native descriptor is not in this manifest"))?;
                let resolver = Arc::new(JobArtifacts::new(self.contexts.clone()));
                let mut resident = self
                    .services
                    .acquire(descriptor, resolver, &invocation.cancellation)
                    .await
                    .map_err(handler)?;
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
                        return Err(handler(resident.failure(error).await));
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
            output.finish(code, leaf.json_output()).await?;
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
    output: &mut CommandOutput,
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
fn handler(error: impl std::fmt::Display) -> ServiceError {
    ServiceError::Handler(error.to_string())
}
