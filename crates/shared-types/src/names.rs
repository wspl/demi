//! The macros every contract crate declares its wire names with: a closed
//! set of names as an enum, and an identifier the model reads and writes,
//! such as a browser tab's `t7` or a simulator's `m2`.

#[doc(hidden)]
pub use regex as __regex;
#[doc(hidden)]
pub use serde_plain as __serde_plain;

/// Declares a closed set of wire names as an enum that displays and parses
/// each value as the wire spells it. The calling crate depends on `serde`
/// and `schemars`.
#[macro_export]
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

        $crate::names::__serde_plain::derive_display_from_serialize!($name);
        $crate::names::__serde_plain::derive_fromstr_from_deserialize!($name);
    };
}

/// Declares an identifier the model reads and writes, a fixed prefix and a
/// number from 1 such as `t7`, as a checked newtype. Its pattern is both its
/// check and its schema; `$what` says what the pattern stands for, in the
/// schema and in the refusal of a value that does not match it. The calling
/// crate depends on `serde` and `schemars`.
#[macro_export]
macro_rules! numbered {
    ($(#[$meta:meta])* $name:ident, $prefix:literal, $what:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
        #[serde(try_from = "String")]
        #[schemars(extend("pattern" = $name::PATTERN, "patternDescription" = $what))]
        pub struct $name(String);

        impl $name {
            /// What an identifier with this prefix looks like: the prefix and
            /// a number from 1, which JavaScript holds exactly.
            pub const PATTERN: &str = concat!("^", $prefix, "[1-9][0-9]{0,14}$");

            /// The identifier of `number`, which is from 1.
            pub fn numbered(number: u64) -> Self {
                debug_assert_ne!(number, 0, "identifiers are numbered from 1");
                Self(format!(concat!($prefix, "{}"), number))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// The number after the prefix.
            pub fn number(&self) -> u64 {
                self.0[$prefix.len()..]
                    .parse()
                    .expect("an identifier's number was checked")
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                static PATTERN: std::sync::LazyLock<$crate::names::__regex::Regex> =
                    std::sync::LazyLock::new(|| {
                        $crate::names::__regex::Regex::new($name::PATTERN)
                            .expect("the identifier pattern compiles")
                    });
                if PATTERN.is_match(&value) {
                    Ok(Self(value))
                } else {
                    Err(format!(concat!("{:?} is not ", $what), value))
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::try_from(value.to_owned())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }
    };
}
