//! The `demi.browser` package (`crates-and-packages.md` § demi-browser): the
//! conversations' browsers, as a resident command service composed of the
//! browser libraries.

mod conversations;

use std::{future::Future, pin::Pin, sync::Arc};

use demi_browser_driver::installation::BrowserDirectories;
use demi_browser_protocol::{Operation, OperationError, browser as protocol};
use demi_command_service::protocol::{Completion, Invocation};
use demi_command_service::{
    ConversationContext, Handler, InvocationContext, Numbers, ServiceError,
};

/// The service.
pub struct DemiBrowser {
    browsers: Arc<conversations::Conversations>,
}

impl DemiBrowser {
    /// The service, whose browsers find or install the pinned Chrome in
    /// `directories`: a Host's are [`BrowserDirectories::host`].
    pub fn new(directories: BrowserDirectories) -> Self {
        Self {
            browsers: Arc::new(conversations::Conversations::new(directories)),
        }
    }
}

impl Handler for DemiBrowser {
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

    /// The browsers number their tabs from the conversations' `tab`
    /// sequences (`browser.md` § One tab registry).
    fn numbers(&self, numbers: Numbers) {
        self.browsers.attach_numbers(numbers);
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
            Ok(Operation::Browser(operation)) => {
                Box::pin(async move { browsers.invoke(context, Ok(operation)).await })
            }
            Ok(Operation::Live) => Box::pin(async move { browsers.live(context).await }),
            Err(OperationError::Unknown(operation)) => {
                Box::pin(async move { Err(ServiceError::UnknownOperation(operation)) })
            }
            Err(error) => Box::pin(async move { browsers.invoke(context, Err(error)).await }),
        }
    }
}
