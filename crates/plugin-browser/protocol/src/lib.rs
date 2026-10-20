//! The `demi.browser` package's contract: the arguments and results of its
//! `browser.*` operations, the live view's protocol, the capture extension's
//! messages, and the pinned Chrome release records. It holds types and their
//! checks only; the operations, transport and IO live in `demi-browser` and the
//! browser libraries.
//!
//! The types are defined once: the `demi browser` declarations take their
//! schemas from them, the handlers decode into them, and the web app uses the
//! TypeScript generated from them. Whatever enters from outside (command input,
//! Host files, CDP replies and events, page tool results, the page's and the
//! extension's messages) is decoded and bounded here, and corrupt data is
//! refused, never repaired.

/// The package's id, which its release descriptor names and the coding
/// agent's commands and the backend's `browser` user stream bind to.
pub const PACKAGE: &str = "demi.browser";

// The wire-name macros every contract crate shares.
#[macro_use]
extern crate demi_shared_types;

pub mod browser;
pub mod capture;
pub mod live;
pub mod release;

use demi_shared_types::DecodeError;

/// An invocation of the package: the operation its name names, with the
/// operation's checked arguments.
#[derive(Debug, Clone, PartialEq)]
pub enum Operation {
    Browser(Box<browser::BrowserOperation>),
    /// `browser.live`: a viewer of the conversation's browser.
    Live,
}

/// Why an invocation could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    /// The name is not the package's: no `browser.` operation.
    #[error("unknown operation {0}")]
    Unknown(String),
    /// A `browser.` operation the package does not serve, which the browser
    /// reports as its own failure.
    #[error("unknown operation {0}")]
    Unserved(String),
    /// The operation's arguments are refused.
    #[error(transparent)]
    Invalid(#[from] DecodeError),
}

impl Operation {
    /// Decodes the operation `name` and its arguments.
    pub fn parse(name: &str, args: serde_json::Value) -> Result<Self, OperationError> {
        if name == live::OPERATION {
            return demi_shared_types::decode_value::<live::LiveInput>(args)
                .map(|_| Self::Live)
                .map_err(OperationError::Invalid);
        }
        match name.strip_prefix(browser::PREFIX) {
            Some(browser) => browser::BrowserOperation::parse(browser, args)
                .map(|operation| Self::Browser(Box::new(operation))),
            None => Err(OperationError::Unknown(name.to_owned())),
        }
    }

    /// Every operation the package serves, as its descriptor lists them: the
    /// browser operations and the live view.
    pub fn names() -> impl Iterator<Item = &'static str> {
        browser::OPERATIONS
            .iter()
            .chain([&live::OPERATION])
            .copied()
    }
}
