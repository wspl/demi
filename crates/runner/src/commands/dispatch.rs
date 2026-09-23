//! The local API validates declarations before routing native and callback work.

use crate::commands::cache::ArtifactResolver;
use crate::{
    commands::command_client::RawCommand,
    commands::command_output::CommandOutput,
    commands::contexts::Contexts,
    commands::native::Services,
    commands::rpc::{self, Calls},
    management::{self, Management},
};
use bytes::Bytes;
use demi_command_service::protocol::{Completion, Invocation, LocalInvocation, Record};
use demi_command_service::{Handler, Input, InvocationContext, ServiceError};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct Dispatcher {
    pub contexts: Contexts,
    pub services: Arc<Services>,
    pub resolver: Arc<dyn ArtifactResolver>,
    pub calls: Arc<Calls>,
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
            let output = CommandOutput::new(invocation.output, parsed.json);
            let _hint = self
                .calls
                .running_hint(&context.job_id, leaf.running_hint.as_deref())
                .await?;
            let code = if let Some(binding) = leaf.binding() {
                let descriptor = context
                    .manifest
                    .packages
                    .get(&binding.descriptor_hash)
                    .ok_or_else(|| handler("native descriptor is not in this manifest"))?;
                let client = self
                    .services
                    .acquire(descriptor, self.resolver.clone(), &invocation.cancellation)
                    .await
                    .map_err(handler)?;
                let request = Invocation {
                    context: context.command.clone(),
                    json: Some(parsed.json),
                    edits: context.edits.get().cloned(),
                    operation: binding.operation.clone(),
                    invocation_id: invocation.request.invocation_id,
                    args: parsed.values.into(),
                    cwd: invocation.request.cwd,
                    env: invocation.request.env,
                };
                let (input, response) = client.invoke(&request).await?;
                native_exchange(input, response, invocation.input, output.clone()).await?
            } else {
                let mut argv = vec![raw.root.clone()];
                argv.extend(raw.argv);
                self.calls
                    .invoke(
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
                        output.clone(),
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

async fn native_exchange(
    mut sender: demi_command_service::CommandInput,
    mut response: demi_command_service::CommandOutput,
    mut input: Input,
    output: CommandOutput,
) -> Result<u8, ServiceError> {
    let (pull, mut demanded) = mpsc::channel::<()>(1);
    let send = async {
        while demanded.recv().await.is_some() {
            match input.next().await? {
                Some(bytes) => sender.write(bytes).await?,
                None => {
                    sender.end()?;
                    return std::future::pending::<Result<u8, ServiceError>>().await;
                }
            }
        }
        std::future::pending::<Result<u8, ServiceError>>().await
    };
    let receive = async {
        let mut completion = None;
        while let Some(record) = response.next().await? {
            match record {
                Record::Stdout(bytes) => output.stdout(bytes).await?,
                Record::Stderr(bytes) => {
                    output.stderr(bytes).await?;
                }
                Record::InputPull => pull
                    .try_send(())
                    .map_err(|_| handler("overlapping native input demands"))?,
                Record::Completion(value) => completion = Some(value),
            }
        }
        let completion = completion.ok_or_else(|| handler("native command has no completion"))?;
        if let Some(error) = completion.error {
            output
                .stderr(Bytes::from(format!("{}: {}\n", error.code, error.message)))
                .await?;
        }
        Ok(completion.exit_code)
    };
    tokio::select! {
        result = send => result,
        result = receive => result,
    }
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
