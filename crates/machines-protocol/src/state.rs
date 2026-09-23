//! A device's stored images (`managed-hosts.md` § Images): the names that
//! identify them and the committed generation's record, which both
//! `image_state` answers and the manager keeps on disk.

use std::{fmt, num::NonZeroU64};

use serde::{Deserialize, Serialize};

/// A name that breaks the image-name rule.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind}: {value:?}")]
pub struct IdError {
    kind: &'static str,
    value: String,
}

/// Whether `value` may name a device, a generation or a base: one or more
/// ASCII letters, digits, `_` or `-`. Such a name is always one path
/// component, so it names a directory under the manager's state directory.
pub fn is_image_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

macro_rules! image_name {
    ($(#[$doc:meta])* $name:ident, $kind:literal) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Checks `value` against the image-name rule ([`is_image_name`]).
            pub fn parse(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if !is_image_name(&value) {
                    return Err(IdError { kind: $kind, value });
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(value: String) -> Result<Self, IdError> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(name: $name) -> Self {
                name.0
            }
        }

        impl AsRef<std::path::Path> for $name {
            fn as_ref(&self) -> &std::path::Path {
                self.0.as_ref()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(&self.0, formatter)
            }
        }
    };
}

image_name!(
    /// A persistent Cloud device, as the backend names it. It names the
    /// device's image and working directories.
    DeviceId,
    "device id"
);
image_name!(
    /// One committed pair of a device's system and home images.
    GenerationId,
    "generation"
);
image_name!(
    /// An imported base: the SHA-256 of its image manifest's bytes.
    BaseVersion,
    "base version"
);

/// A device's committed generation: its base, the reset that made it, and
/// the capacities of its two filesystems. The same record answers
/// `image_state`, names the generation in `current.json`, and describes the
/// generation and the working pair on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MachineImageState {
    pub generation: GenerationId,
    pub base_version: BaseVersion,
    /// The reset operation that published this generation's system image,
    /// or `null` before the first reset. The key is always written.
    #[serde(deserialize_with = "Option::deserialize")]
    pub reset_id: Option<String>,
    /// The system filesystem's capacity in bytes, not its use.
    pub system_bytes: NonZeroU64,
    /// The home filesystem's capacity in bytes, not its use.
    pub home_bytes: NonZeroU64,
}

impl MachineImageState {
    /// The recorded capacity of `volume`.
    pub fn bytes(&self, volume: Volume) -> NonZeroU64 {
        match volume {
            Volume::System => self.system_bytes,
            Volume::Home => self.home_bytes,
        }
    }

    /// The record with `volume`'s capacity replaced.
    pub fn with_bytes(mut self, volume: Volume, bytes: NonZeroU64) -> Self {
        match volume {
            Volume::System => self.system_bytes = bytes,
            Volume::Home => self.home_bytes = bytes,
        }
        self
    }
}

/// Whether the manager runs a sandbox for a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Running,
    Stopped,
}

/// One of a device's two writable filesystems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Volume {
    /// The system layer: the OverlayFS upper and work directories.
    System,
    /// `/home`.
    Home,
}

impl Volume {
    /// Both volumes, system first.
    pub const ALL: [Volume; 2] = [Self::System, Self::Home];
}

serde_plain::derive_display_from_serialize!(RuntimeState);
serde_plain::derive_fromstr_from_deserialize!(RuntimeState);
serde_plain::derive_display_from_serialize!(Volume);
serde_plain::derive_fromstr_from_deserialize!(Volume);
