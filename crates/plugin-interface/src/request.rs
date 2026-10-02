//! What the plugin host asks of a plugin, and what the plugin answers
//! (`plugins.md` § The contract). Each request names its user, since a
//! plugin process would serve every user.

use demi_host_interface::{PortError, RpcError, RpcInvocation};
use demi_shared_types::{NodeId, TurnId};
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::panel::PanelTab;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{PortFailure, PortRefusal, Topic};

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
        user: UserId,
        invocation: Box<RpcInvocation>,
    },
    /// A node is about to send a provider request, and the plugin, a
    /// context source, may add a context block (`plugins.md` § Prompt text
    /// and context): `seen` holds the text of its own blocks the model
    /// receives, oldest first, and `cwd` is the node's working directory.
    Context {
        user: UserId,
        conversation: ConversationId,
        node: NodeId,
        cwd: String,
        turn: TurnId,
        seen: Vec<String>,
    },
    /// The plugin's page state: the user's, or, with `conversation`, that
    /// conversation's.
    PageState {
        user: UserId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        conversation: Option<ConversationId>,
    },
    /// A page called a method, with parameters that are valid against the
    /// method's schema; `conversation` is the conversation a method of the
    /// conversation scope was called for.
    PageCall {
        user: UserId,
        method: String,
        params: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        conversation: Option<ConversationId>,
    },
    /// The user created or removed a tab of one of the plugin's panel
    /// kinds; the backend applied the change and answered the page
    /// (`plugins.md` § Panel kinds). `tab` is the tab as it was created, or
    /// as it was when it was removed.
    PanelTab {
        user: UserId,
        conversation: ConversationId,
        change: PanelTabChange,
        tab: PanelTab,
    },
    /// A topic the plugin is told about fired, for the user or for
    /// `conversation` (`plugins.md` § Topics).
    Topic {
        user: UserId,
        topic: Topic,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        conversation: Option<ConversationId>,
    },
}

/// What the user did to a tab of the plugin's panel kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelTabChange {
    Created,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Reply {
    /// A command's exit status.
    Exit { code: u8 },
    /// A context request's new block; none to add nothing.
    Context {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// The page state, valid against its scope's declared schema.
    State { state: Value },
    /// A page call's result, valid against the method's result schema.
    Result { result: Value },
    /// A request that answers nothing, such as a `panel_tab` or a `topic`.
    Done,
}

/// Why a plugin gave up on a request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
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
    /// The plugin refuses a page call: `reason` is a snake_case word, such
    /// as `tab_not_found`, which the page sees with the message.
    #[error("{message}")]
    Refused { reason: String, message: String },
    /// A port operation was refused, and the plugin passes the refusal on:
    /// the caller sees the refusal's own answer.
    #[error("{refusal}")]
    Port { refusal: PortRefusal },
    /// The plugin failed.
    #[error("{message}")]
    Failed { message: String },
    /// The request is over: cancelled, or its caller went away.
    #[error("{message}")]
    Ended { message: String },
}

impl PluginError {
    pub fn refused(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Refused {
            reason: reason.into(),
            message: message.into(),
        }
    }

    pub fn failed(message: impl ToString) -> Self {
        Self::Failed {
            message: message.to_string(),
        }
    }

    /// The answer to a request for something the plugin's manifest does
    /// not declare, such as `"a page state"`, which the plugin host never
    /// sends.
    pub fn undeclared(what: &str) -> Self {
        Self::failed(format!("the plugin declares no {what}"))
    }
}

impl From<RpcError> for PluginError {
    fn from(error: RpcError) -> Self {
        match error {
            RpcError::Usage(message) => Self::Usage { message },
            RpcError::Failed(message) => Self::Failed { message },
            RpcError::Port(error) => error.into(),
        }
    }
}

impl From<PortError> for PluginError {
    fn from(error: PortError) -> Self {
        match error {
            PortError::Ended(message) => Self::Ended { message },
            error => Self::failed(error),
        }
    }
}

impl From<PortFailure> for PluginError {
    fn from(failure: PortFailure) -> Self {
        match failure {
            PortFailure::Port(error) => error.into(),
            PortFailure::Refused(refusal) => Self::Port { refusal },
        }
    }
}

impl From<PluginError> for RpcError {
    fn from(error: PluginError) -> Self {
        match error {
            PluginError::Usage { message } => Self::Usage(message),
            PluginError::Ended { message } => Self::Port(PortError::Ended(message)),
            error => Self::Failed(error.to_string()),
        }
    }
}
