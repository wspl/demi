//! The `demi.browser` package's contract (`crates-and-packages.md`
//! § command-package-browser-protocol): the arguments and results of its `browser.*`
//! operations, the live view's protocol, the capture extension's messages,
//! and the pinned Chrome release records. It holds types and their checks
//! only; the operations, transport and IO live in `demi-browser` and the
//! browser libraries.

/// The package's id, which its release descriptor names and the coding
/// agent's commands and the backend's `browser` user stream bind to.
pub const PACKAGE: &str = "demi.browser";

/// Declares a closed set of wire names as an enum that displays and parses
/// each value as the wire spells it.
macro_rules! closed_set {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($(#[$variant_meta:meta])* $variant:ident = $wire:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
            schemars::JsonSchema,
        )]
        pub enum $name {
            $($(#[$variant_meta])* #[serde(rename = $wire)] $variant,)*
        }

        serde_plain::derive_display_from_serialize!($name);
        serde_plain::derive_fromstr_from_deserialize!($name);
    };
}

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
