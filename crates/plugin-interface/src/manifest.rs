//! What a plugin declares once, when the backend starts (`plugins.md`
//! § What a plugin contributes): fixed for the life of the process.

use std::fmt;

use demi_command_declarations::{NativeOperation, Node};
use demi_shared_types::Profile;
use serde::{Deserialize, Serialize};

/// The context source the product's execution context answers as, which no
/// plugin may be named.
pub const EXECUTION_SOURCE: &str = "execution";

/// The root the plugins of this repository place their groups under.
pub const DEMI_ROOT: &str = "demi";

/// The `demi` root's summary in the model's command help.
pub const DEMI_SUMMARY: &str =
    "The Demi platform command: every subcommand is a platform domain (file, todo, …).";

/// A plugin's id, such as `todo`: 1 to 32 lowercase letters, digits and
/// hyphens. It names the plugin's context blocks, values, part of the
/// product state, directories on a Host and page route.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginId(String);

/// Why a text is not a plugin's id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "\"{0}\" is not a plugin id: 1 to 32 lowercase letters, digits and hyphens, not \"{EXECUTION_SOURCE}\""
)]
pub struct PluginIdError(String);

impl PluginId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PluginId {
    type Error = PluginIdError;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        let valid = (1..=32).contains(&id.len())
            && id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && id != EXECUTION_SOURCE;
        if valid {
            Ok(Self(id))
        } else {
            Err(PluginIdError(id))
        }
    }
}

impl TryFrom<&str> for PluginId {
    type Error = PluginIdError;

    fn try_from(id: &str) -> Result<Self, Self::Error> {
        Self::try_from(id.to_owned())
    }
}

impl From<PluginId> for String {
    fn from(id: PluginId) -> Self {
        id.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A plugin's declarations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub id: PluginId,
    /// Its command groups and roots (`plugins.md` § Commands).
    #[serde(default)]
    pub commands: Vec<Commands>,
    /// Its subagent profiles (`plugins.md` § Profiles).
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

impl Manifest {
    /// A manifest that declares nothing yet.
    pub fn new(id: PluginId) -> Self {
        Self {
            id,
            commands: Vec::new(),
            profiles: Vec::new(),
        }
    }
}

/// One command tree of a plugin, and where it goes in the command set every
/// node starts from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Commands {
    pub placement: Placement,
    /// The tree, as data: an `rpc` leaf reaches the plugin as a command
    /// request, a native leaf runs its operation on the Host.
    pub tree: Node<NativeOperation>,
}

/// Where a plugin's command tree goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// A named group of the `demi` root, for a plugin of this repository.
    Demi,
    /// A root command of its own.
    Root,
}
