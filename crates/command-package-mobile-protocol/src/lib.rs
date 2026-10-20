//! The `demi.mobile` package's contract.
//!
//! The crate holds:
//!
//! - the arguments and results of the `mobile.*` operations, and their
//!   failures;
//! - the message the package sends its plugin;
//! - the live view's protocol for a simulator.
//!
//! It holds types and their checks only; the operations, the drivers and their
//! IO live in `demi-mobile`.

#[macro_use]
extern crate demi_shared_types;

pub mod live;
pub mod mobile;

use demi_shared_types::DecodeError;

/// The package's id, which its release descriptor names and the agent's
/// `demi mobile` commands and the `mobile` user stream bind to.
pub const PACKAGE: &str = "demi.mobile";

/// An invocation of the package: the operation its name names, with the
/// operation's checked arguments.
#[derive(Debug, Clone, PartialEq)]
pub enum Operation {
    Mobile(Box<mobile::MobileOperation>),
    /// `mobile.live`: a viewer of the conversation's devices.
    Live,
}

/// Why an invocation could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    /// No operation of the package has the name.
    #[error("unknown operation {0}")]
    Unknown(String),
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
        match name.strip_prefix(mobile::PREFIX) {
            Some(operation) => mobile::MobileOperation::parse(operation, args)
                .map(|operation| Self::Mobile(Box::new(operation))),
            None => Err(OperationError::Unknown(name.to_owned())),
        }
    }

    /// Every operation the package serves, as its descriptor lists them.
    pub fn names() -> impl Iterator<Item = &'static str> {
        mobile::OPERATIONS
            .iter()
            .chain([&live::OPERATION])
            .copied()
    }
}
