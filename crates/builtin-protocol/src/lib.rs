//! The `demi.builtin` package's contract (`crates-and-packages.md`
//! § builtin-protocol): the arguments and results of its `file.*` and
//! `browser.*` operations, the live view's protocol, the capture extension's
//! messages, and the pinned Chrome release records. It holds types and their
//! checks only; the operations, transport and IO live in `demi-commands`.

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
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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

/// Every operation the package serves: the file operations, the browser
/// operations and the live view.
pub fn operations() -> impl Iterator<Item = &'static str> {
    file::OPERATIONS
        .iter()
        .chain(browser::OPERATIONS)
        .chain([&live::OPERATION])
        .copied()
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
