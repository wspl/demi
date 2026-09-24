//! Declared builtins call the resident dispatcher without a transport hop.

use super::{invocation_file, scope::Scope};
use crate::commands::command_client::RawCommand;
use brush_core::{CommandArg, ExecutionContext, ExecutionResult, builtins};
use bytes::Bytes;
use demi_command_service::{
    Handler, Input, InvocationContext, Output, ServiceError,
    protocol::{LocalInvocation, Record},
};
use std::{collections::BTreeMap, io, sync::Arc};

pub(super) fn execute(
    context: ExecutionContext<'_>,
    args: Vec<CommandArg>,
) -> builtins::BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
    Box::pin(async move {
        let scope = context
            .shell
            .execution_host()
            .and_then(|host| (host.as_ref() as &dyn std::any::Any).downcast_ref::<Scope>())
            .ok_or_else(|| io::Error::other("missing shell execution owner"))?
            .clone();
        let commands = scope
            .commands
            .as_ref()
            .ok_or_else(|| io::Error::other("missing command context"))?;
        let env: BTreeMap<_, _> = context
            .shell
            .env()
            .iter_exported()
            .filter(|(_, variable)| variable.value().is_set())
            .map(|(name, variable)| {
                (
                    name.clone(),
                    variable.value().to_cow_str(context.shell).into_owned(),
                )
            })
            .collect();
        let stdin = Arc::new(invocation_file(&context, 0)?);
        let stdout = Arc::new(invocation_file(&context, 1)?);
        let stderr = Arc::new(invocation_file(&context, 2)?);
        let raw = RawCommand::new(
            commands.execution.id.clone(),
            context.command_name,
            args.into_iter().skip(1).map(|arg| arg.to_string()).collect(),
            crate::stdio::is_live(&stdin, &env)?,
        )?;
        let cancellation = scope.cancellation.child_token();
        let _cancel_on_return = cancellation.drop_guard_ref();
        let input_scope = scope.with_cancellation(cancellation.clone());
        let input = Input::from_stream(futures_util::stream::try_unfold(
            (stdin, input_scope),
            |(file, scope)| async move {
                let worker_file = file.clone();
                let worker_scope = scope.clone();
                let bytes = scope
                    .tasks
                    .spawn_blocking(move || -> io::Result<Bytes> {
                        let mut bytes = vec![0; 64 * 1024];
                        let count = worker_scope.read(&worker_file, &mut bytes)?;
                        bytes.truncate(count);
                        Ok(Bytes::from(bytes))
                    })
                    .await??;
                Ok::<_, ServiceError>((!bytes.is_empty()).then_some((bytes, (file, scope))))
            },
        ));
        let (output, mut records) = Output::channel(cancellation.clone());
        let invocation = commands.dispatcher.invoke(InvocationContext {
            request: LocalInvocation {
                operation: "raw".into(),
                invocation_id: uuid::Uuid::new_v4().simple().to_string(),
                args: serde_json::to_value(raw).map_err(io::Error::other)?,
                cwd: context.shell.working_dir().to_string_lossy().into_owned(),
                env,
            },
            input,
            output,
            cancellation: cancellation.clone(),
        });
        let drain = async {
            while let Some(record) = records.recv().await {
                let (file, mut bytes) = match record {
                    Record::Stdout(bytes) => (stdout.clone(), bytes),
                    Record::Stderr(bytes) => (stderr.clone(), bytes),
                    _ => return Err(io::Error::other("unexpected local output record")),
                };
                let writer_scope = scope.clone();
                scope
                    .tasks
                    .spawn_blocking(move || -> io::Result<()> {
                        while !bytes.is_empty() {
                            let count = writer_scope.write(&file, &bytes)?;
                            if count == 0 {
                                return Err(io::ErrorKind::WriteZero.into());
                            }
                            bytes = bytes.slice(count..);
                        }
                        Ok(())
                    })
                    .await
                    .map_err(io::Error::other)??;
            }
            Ok::<_, io::Error>(())
        };
        let (completion, ()) =
            tokio::try_join!(async { invocation.await.map_err(io::Error::other) }, drain)?;
        Ok(brush_core::ExecutionExitCode::from(completion.exit_code).into())
    })
}
