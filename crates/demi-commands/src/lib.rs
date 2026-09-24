//! Execution-target implementations for Demi builtin commands.

pub mod browser;

mod files;
mod patch;

use std::{future::Future, pin::Pin, sync::Arc};

use demi_builtin_protocol::{Operation, OperationError};
use demi_command_service::protocol::{Completion, Invocation};
use demi_command_service::{ConversationContext, Handler, InvocationContext, ServiceError};
use demi_gates::SerialGate;

#[derive(Default)]
pub struct DemiCommands {
    /// File mutations run one at a time.
    mutations: SerialGate,
    browsers: Arc<browser::Conversations>,
}

impl Handler for DemiCommands {
    type Metadata = Invocation;

    fn conversation(
        &self,
        context: ConversationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let browsers = self.browsers.clone();
        Box::pin(async move { browsers.conversation(context).await })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), ServiceError>> + Send>> {
        let browsers = self.browsers.clone();
        Box::pin(async move { browsers.close().await })
    }

    fn operations(&self) -> Vec<String> {
        Operation::names().map(String::from).collect()
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let browsers = self.browsers.clone();
        match Operation::parse(&context.request.operation, context.request.args.clone()) {
            Ok(Operation::File(operation)) => {
                Box::pin(files::invoke(context, operation, self.mutations.clone()))
            }
            Err(OperationError::File(error)) => {
                Box::pin(async move { Ok(files::failure(&error.into())) })
            }
            Ok(Operation::Browser(operation)) => {
                Box::pin(async move { browsers.invoke(context, Ok(operation)).await })
            }
            Err(OperationError::Browser(error)) => {
                Box::pin(async move { browsers.invoke(context, Err(error)).await })
            }
            Ok(Operation::Live) => Box::pin(async move { browsers.live(context).await }),
            Err(OperationError::Unknown(operation)) => {
                Box::pin(async move { Err(ServiceError::UnknownOperation(operation)) })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// The release tooling reads the package's id from the crate's Cargo
    /// metadata; the contract's id is the one the coding agent binds to.
    #[test]
    fn the_cargo_metadata_names_the_contracts_package() {
        let manifest = include_str!("../Cargo.toml");
        let metadata = manifest
            .split_once("[package.metadata.demi]")
            .expect("the crate declares its package")
            .1;
        let id = metadata
            .lines()
            .find_map(|line| line.strip_prefix("id = "))
            .expect("the package has an id");
        assert_eq!(id, format!("\"{}\"", demi_builtin_protocol::PACKAGE));
    }
}
