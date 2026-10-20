//! The `demi.fetch` package (`crates-and-packages.md` § command-package-fetch):
//! `demi web fetch` reads a URL from the Host, as `curl` there would, and
//! prints a page as Markdown one part at a time, or saves a file that is no
//! text. It keeps the pages it read on disk for 15 minutes and holds no
//! conversation state, so the SDK's empty conversation status, release and
//! close stand, and the service ends with its last lease.

mod cache;
mod download;
mod get;
mod markdown;
mod saved;

use std::{future::Future, pin::Pin};

use demi_command_package_fetch_protocol::{OPERATIONS, Operation, OperationError};
use demi_command_protocol::{Completion, Invocation};
use demi_command_sdk::{Handler, InvocationContext, ServiceError};

/// The service.
pub struct DemiFetch;

impl Handler for DemiFetch {
    type Metadata = Invocation;

    fn operations(&self) -> Vec<String> {
        OPERATIONS.iter().map(|&name| name.to_owned()).collect()
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        match Operation::parse(&context.request.operation, context.request.args.clone()) {
            Ok(Operation::Get(args)) => Box::pin(get::get(context, args)),
            Err(OperationError::Invalid(error)) => {
                Box::pin(async move { Ok(get::failure(error.to_string())) })
            }
            Err(OperationError::Unknown(operation)) => {
                Box::pin(async move { Err(ServiceError::UnknownOperation(operation)) })
            }
        }
    }
}
