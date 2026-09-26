//! The `demi.builtin` package's contract (`crates-and-packages.md`
//! § builtin-protocol): the arguments and results of its `file.*` and
//! `browser.*` operations, the live view's protocol, the capture extension's
//! messages, and the pinned Chrome release records. It holds types and their
//! checks only; the operations, transport and IO live in `demi-commands`.

/// The package's id, which its release descriptor names and the coding
/// agent's commands bind to.
pub const PACKAGE: &str = "demi.builtin";

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
pub mod file;
pub mod live;
pub mod release;

use serde::de::DeserializeOwned;

/// Why a value that entered the process was refused.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The value does not have the type's shape.
    #[error(transparent)]
    Shape(#[from] serde_json::Error),
    /// The value has the shape but breaks one of the type's rules.
    #[error("{0}")]
    Invalid(String),
    #[error("unknown operation {0}")]
    UnknownOperation(String),
}

impl From<garde::Report> for DecodeError {
    fn from(report: garde::Report) -> Self {
        Self::Invalid(report.to_string().trim_end().to_owned())
    }
}

/// An invocation of the package: the operation its name names, with the
/// operation's checked arguments.
#[derive(Debug, Clone, PartialEq)]
pub enum Operation {
    File(file::FileOperation),
    Browser(browser::BrowserOperation),
    /// `browser.live`: a viewer of the conversation's browser.
    Live,
}

/// Why an invocation could not be decoded. The part of the package whose
/// operation it names reports the failure, each in its own way.
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    #[error("unknown operation {0}")]
    Unknown(String),
    #[error(transparent)]
    File(DecodeError),
    #[error(transparent)]
    Browser(DecodeError),
}

impl Operation {
    /// Decodes the operation `name` and its arguments.
    pub fn parse(name: &str, args: serde_json::Value) -> Result<Self, OperationError> {
        if name == live::OPERATION {
            return decode::<live::LiveInput>(args)
                .map(|_| Self::Live)
                .map_err(OperationError::Browser);
        }
        if let Some(browser) = name.strip_prefix(browser::PREFIX) {
            return browser::BrowserOperation::parse(browser, args)
                .map(Self::Browser)
                .map_err(OperationError::Browser);
        }
        match file::FileOperation::parse(name, args) {
            Ok(operation) => Ok(Self::File(operation)),
            Err(DecodeError::UnknownOperation(name)) => Err(OperationError::Unknown(name)),
            Err(error) => Err(OperationError::File(error)),
        }
    }

    /// Every operation the package serves, as its descriptor lists them: the
    /// file operations, the browser operations and the live view.
    pub fn names() -> impl Iterator<Item = &'static str> {
        file::OPERATIONS
            .iter()
            .chain(browser::OPERATIONS)
            .chain([&live::OPERATION])
            .copied()
    }
}

/// Decodes a JSON value that entered the process and checks its rules.
fn decode<T>(value: serde_json::Value) -> Result<T, DecodeError>
where
    T: DeserializeOwned + garde::Validate<Context = ()>,
{
    let decoded: T = serde_json::from_value(value)?;
    decoded.validate()?;
    Ok(decoded)
}

/// Decodes JSON text that entered the process and checks its rules.
fn decode_slice<T>(bytes: &[u8]) -> Result<T, DecodeError>
where
    T: DeserializeOwned + garde::Validate<Context = ()>,
{
    let decoded: T = serde_json::from_slice(bytes)?;
    decoded.validate()?;
    Ok(decoded)
}
