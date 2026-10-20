//! The `demi.browser` package: the browsers of the conversations, as a resident
//! command service that carries out the `demi browser` catalog.
//!
//! The browser is a plugin (`plugin-browser`) whose commands bind this package
//! through the conversation-scoped state port. Nothing in the agent, the
//! backend, the runner or their wire protocols holds browser logic: they carry
//! the invocation, its command context, the conversation release and the user
//! stream, and name no browser operation. `plugin-browser`'s tab methods and
//! `browser` user stream only name this package's operations.
//!
//! ```text
//! Agent shell call
//!    -> Brush on the Host -> declared-command dispatcher (runner)
//!         -> this service: one conversation browser per conversation
//!              -> Chrome for Testing over CDP
//!    <- stdout, stderr, exit status -> shell result and media handling
//! ```
//!
//! - The conversation and the invoking agent come from the command context,
//!   which a script cannot change; a command has no `--conversation`,
//!   `--session`, `--profile` or `--cdp-url`.
//! - A conversation has one environment per Host where its shell runs, and a
//!   command never reaches the one on another Host (its handle is
//!   `tab_not_found` there). The service does not know which role its Host
//!   plays; the conversation release reaches every device the conversation
//!   used.
//! - Isolation between conversations is a product rule, not a security boundary
//!   against a shell on the Host.
//! - Refused: discovering a Chrome on `PATH`, attaching to a user's browser, a
//!   batch language or REPL of its own (Bash composes the commands), and a
//!   second shell or model-tool loop.
//! - Where Codex's browser API and Playwright differ, Codex's verified normal
//!   behavior decides; an obvious defect of the reference is not adopted.
//!
//! It is a program of its own: it hosts no runner connection, defines no part
//! of the agent's command tree, stores no conversation, and is never linked
//! into the runner.
//!
//! Its runtime is multi-threaded because several conversations' browsers must
//! make progress at once, and the Chrome DevTools connection decodes large
//! messages (a screenshot is megabytes of base64 JSON) on whichever worker
//! polls it. Owner tasks take the place of locks, so a thread per conversation
//! would add only a hop and pin a conversation's commands, live view frames and
//! DevTools decoding to one thread.

// Whether the service is `Send` and `Sync` is decided through a conversation's
// browser and the channels that answer with it, deeper than the default 128
// steps of the trait solver.
#![recursion_limit = "256"]

mod conversations;

use std::{future::Future, pin::Pin, sync::Arc};

use demi_command_package_browser_chrome::driver::installation::Chrome;
use demi_command_package_browser_protocol::{Operation, OperationError, browser as protocol};
use demi_command_protocol::{Completion, Invocation};
use demi_command_sdk::{
    Artifacts, ConversationContext, Handler, InvocationContext, Messages, Numbers, ServiceError,
};

/// The service.
pub struct DemiBrowser {
    browsers: Arc<conversations::Conversations>,
    chrome: Chrome,
}

impl Default for DemiBrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl DemiBrowser {
    /// The service, whose browsers start the pinned Chrome, which the
    /// runner installs.
    pub fn new() -> Self {
        Self::with_chrome(Chrome::default())
    }

    /// The service, whose browsers start `chrome`.
    pub fn with_chrome(chrome: Chrome) -> Self {
        Self {
            browsers: Arc::new(conversations::Conversations::new(chrome.clone())),
            chrome,
        }
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
        Box::pin(async move { browsers.close().await })
    }

    /// The browsers number their tabs from the conversations' `tab`
    /// sequences.
    fn numbers(&self, numbers: Numbers) {
        self.browsers.attach_numbers(numbers);
    }

    /// The browsers tell the plugin when a conversation's tabs change.
    fn messages(&self, messages: Messages) {
        self.browsers.attach_messages(messages);
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
            Err(OperationError::Unknown(operation)) => {
                Box::pin(async move { Err(ServiceError::UnknownOperation(operation)) })
            }
            Err(error) => Box::pin(async move { browsers.invoke(context, Err(error)).await }),
        }
    }
}
