//! The `demi.file` package (`crates-and-packages.md` § command-package-file): reading,
//! creating, editing and patching files beside them, as a resident command
//! service. It holds no conversation state, so the SDK's empty conversation
//! status, release and close stand, and the service ends with its last
//! lease.

mod files;
mod patch;

use std::{future::Future, pin::Pin};

use demi_command_package_file_protocol::{OPERATIONS, Operation, OperationError};
use demi_command_protocol::{Completion, Invocation};
use demi_command_sdk::{Handler, InvocationContext, ServiceError};
use demi_shared_gates::SerialGate;

/// The service.
#[derive(Default)]
pub struct DemiFile {
    /// File mutations run one at a time.
    mutations: SerialGate,
}

impl Handler for DemiFile {
    type Metadata = Invocation;

    fn operations(&self) -> Vec<String> {
        OPERATIONS.iter().map(|&name| name.to_owned()).collect()
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        match Operation::parse(&context.request.operation, context.request.args.clone()) {
            Ok(operation) => Box::pin(files::invoke(context, operation, self.mutations.clone())),
            Err(OperationError::Invalid(error)) => {
                Box::pin(async move { Ok(files::failure(&error.into())) })
            }
            Err(OperationError::Unknown(operation)) => {
                Box::pin(async move { Err(ServiceError::UnknownOperation(operation)) })
            }
        }
    }
}
