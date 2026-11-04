//! What an integration's agents may do
//! (`crates/backend-user-shard/src/integrations.rs`), which its creator
//! switches in its editor and the integration through its API.

use std::num::NonZeroU32;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The capabilities an integration turns on or off for its agents. Each is
/// a group of commands, or for subagents the `demi agent` group's spawning,
/// or for titles the request that writes a conversation's title; what an
/// integration never has, such as credentials, is no field here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct IntegrationCapabilities {
    /// Its agents spawn subagents.
    pub subagents: bool,
    /// The browser plugin's commands.
    pub browser: bool,
    /// Web search and fetching.
    pub web: bool,
    /// The skills of the integration's own skill sources.
    pub skills: bool,
    /// Images, videos and audio its agents make.
    pub media: bool,
    /// iOS simulators and Android emulators.
    pub mobile: bool,
    /// Schedules that wake a conversation.
    pub schedules: bool,
    /// Files its agents attach to their replies.
    pub attachments: bool,
    /// A title for each conversation, which the user's small model writes
    /// after its first message, as for the user's own conversations.
    pub titles: bool,
}

/// How every Cloud of an integration is sized, from its next start, and the
/// shared folder they mount; a field that is none is the server's: the
/// Cloud manager's CPU budget and memory limit, and a Cloud's default disks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudSizing {
    /// Its CPU budget, in CPUs.
    #[serde(default)]
    #[garde(skip)]
    pub cpus: Option<CloudLimit>,
    /// Its memory limit, in MiB.
    #[serde(default)]
    #[garde(skip)]
    pub memory_mib: Option<CloudLimit>,
    /// The size of its system disk, in GB.
    #[serde(default)]
    #[schemars(range(min = 1))]
    #[garde(inner(range(min = 1)))]
    pub system_gb: Option<u16>,
    /// The size of its home disk, in GB.
    #[serde(default)]
    #[schemars(range(min = 1))]
    #[garde(inner(range(min = 1)))]
    pub home_gb: Option<u16>,
    /// The size of the integration's shared folder, in GB, which it takes
    /// at its next mount, once no Cloud of the integration runs.
    #[serde(default)]
    #[schemars(range(min = 1))]
    #[garde(inner(range(min = 1)))]
    pub shared_gb: Option<u16>,
}

/// A CPU budget, in CPUs, or a memory limit, in MiB, that an integration
/// names: a number, or `"unlimited"` for none, the host's all.
///
/// It is read by hand, so a wrong value says what a right one is, where an
/// untagged enum's reading says only that no variant matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum CloudLimit {
    At(NonZeroU32),
    Unlimited(Unlimited),
}

impl<'de> Deserialize<'de> for CloudLimit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = CloudLimit;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a whole number of at least 1, or \"unlimited\"")
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<CloudLimit, E> {
                u32::try_from(value)
                    .ok()
                    .and_then(NonZeroU32::new)
                    .map(CloudLimit::At)
                    .ok_or_else(|| E::invalid_value(serde::de::Unexpected::Unsigned(value), &self))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<CloudLimit, E> {
                match u64::try_from(value) {
                    Ok(value) => self.visit_u64(value),
                    Err(_) => Err(E::invalid_value(serde::de::Unexpected::Signed(value), &self)),
                }
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<CloudLimit, E> {
                match value {
                    "unlimited" => Ok(CloudLimit::Unlimited(Unlimited::Unlimited)),
                    _ => Err(E::invalid_value(serde::de::Unexpected::Str(value), &self)),
                }
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// The word that names no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Unlimited {
    Unlimited,
}

impl CloudLimit {
    /// The limit, none for none.
    pub fn value(self) -> Option<NonZeroU32> {
        match self {
            Self::At(value) => Some(value),
            Self::Unlimited(_) => None,
        }
    }
}

impl std::fmt::Display for CloudLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::At(value) => write!(f, "{value}"),
            Self::Unlimited(_) => f.write_str("unlimited"),
        }
    }
}

impl Default for IntegrationCapabilities {
    /// Every capability on: a new integration's agents do what a user's do,
    /// short of what no integration has.
    fn default() -> Self {
        Self {
            subagents: true,
            browser: true,
            web: true,
            skills: true,
            media: true,
            mobile: true,
            schedules: true,
            attachments: true,
            titles: true,
        }
    }
}
