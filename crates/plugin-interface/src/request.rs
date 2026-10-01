//! What the plugin host asks of a plugin, and what the plugin answers
//! (`plugins.md` § The contract). Each request names its user, since a
//! plugin process would serve every user.

use demi_host_interface::{PortError, RpcError, RpcInvocation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Request {
    /// The model ran an `rpc` leaf of the plugin's commands. The
    /// invocation's path starts at the plugin's own tree: a group the plugin
    /// placed under `demi` arrives without the `demi` before it.
    Command {
        user: String,
        invocation: RpcInvocation,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Reply {
    /// A command's exit status.
    Exit { code: u8 },
}

/// Why a plugin gave up on a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PluginError {
    /// The request does not fit, such as a command's arguments its input
    /// refuses.
    #[error("{message}")]
    Usage { message: String },
    /// The plugin failed.
    #[error("{message}")]
    Failed { message: String },
    /// The request is over: cancelled, or its caller went away.
    #[error("{message}")]
    Ended { message: String },
}

impl From<RpcError> for PluginError {
    fn from(error: RpcError) -> Self {
        match error {
            RpcError::Usage(message) => Self::Usage { message },
            RpcError::Failed(message) => Self::Failed { message },
            RpcError::Port(PortError::Ended(message)) => Self::Ended { message },
            RpcError::Port(error) => Self::Failed {
                message: error.to_string(),
            },
        }
    }
}

impl From<PluginError> for RpcError {
    fn from(error: PluginError) -> Self {
        match error {
            PluginError::Usage { message } => Self::Usage(message),
            PluginError::Failed { message } => Self::Failed(message),
            PluginError::Ended { message } => Self::Port(PortError::Ended(message)),
        }
    }
}
