//! The `demi.browser` package (`crates-and-packages.md` § command-package-browser): the
//! conversations' browsers and the Host's web preview engine, as a resident
//! command service composed of the browser libraries.

// Whether the service is `Send` and `Sync` is decided through a conversation's
// browser and the channels that answer with it, deeper than the default 128
// steps of the trait solver.
#![recursion_limit = "256"]

mod conversations;
mod preview;

use std::{future::Future, pin::Pin, sync::Arc};

use demi_command_package_browser_chrome::driver::installation::Chrome;
use demi_command_package_browser_protocol::{Operation, OperationError, browser as protocol};
use demi_command_protocol::{Completion, Invocation};
use demi_command_sdk::{
    Artifacts, ConversationContext, Handler, InvocationContext, Numbers, ServiceError,
};

/// The service.
pub struct DemiBrowser {
    browsers: Arc<conversations::Conversations>,
    chrome: Chrome,
    previews: Arc<preview::Previews>,
}

impl Default for DemiBrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl DemiBrowser {
    /// The service, whose browsers start the pinned Chrome, which the
    /// runner installs (`browser.md` § Browser distribution).
    pub fn new() -> Self {
        Self::with_chrome(Chrome::default())
    }

    /// The service, whose browsers start `chrome`.
    pub fn with_chrome(chrome: Chrome) -> Self {
        Self {
            browsers: Arc::new(conversations::Conversations::new(chrome.clone())),
            chrome,
            previews: Arc::new(preview::Previews::new(None)),
        }
    }

    /// The service, keeping what outlives it on the Host, the web
    /// preview's cookie jar, in `directory`: the package's data directory,
    /// which the runner names when it starts the program.
    pub fn with_data_directory(mut self, directory: std::path::PathBuf) -> Self {
        self.previews = Arc::new(preview::Previews::new(Some(directory)));
        self
    }

    /// The Chrome the browsers start, for finding what its processes left.
    pub fn chrome(&self) -> &Chrome {
        &self.chrome
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
        let previews = self.previews.clone();
        Box::pin(async move {
            let browsers = browsers.close().await;
            previews.close().await?;
            browsers
        })
    }

    /// The browsers number their tabs from the conversations' `tab`
    /// sequences (`browser.md` § One tab registry).
    fn numbers(&self, numbers: Numbers) {
        self.browsers.attach_numbers(numbers);
    }

    fn artifacts(&self, artifacts: Artifacts) {
        self.chrome.attach(artifacts);
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
                Box::pin(async move { browsers.invoke(context, Ok(*operation)).await })
            }
            Ok(Operation::Live) => Box::pin(async move { browsers.live(context).await }),
            Ok(Operation::Preview) => {
                let previews = self.previews.clone();
                Box::pin(async move { previews.serve(context, browsers).await })
            }
            Ok(Operation::PreviewOpen(input)) => Box::pin(async move { preview::open(context, &input).await }),
            Ok(Operation::Handover(input)) => {
                let previews = self.previews.clone();
                Box::pin(async move { previews.handover(context, &input, browsers).await })
            }
            Err(OperationError::Unknown(operation)) => {
                Box::pin(async move { Err(ServiceError::UnknownOperation(operation)) })
            }
            Err(error) => Box::pin(async move { browsers.invoke(context, Err(error)).await }),
        }
    }
}
