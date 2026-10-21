//! What the plugin host asks of a plugin, and what the plugin answers. Each request names its user,
//! since a plugin process would serve every user.

use demi_host_interface::{HostPaths, PortError, RpcError, RpcInvocation};
use demi_shared_types::{NodeId, TurnId};
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::panel::PanelTab;
use demi_web_api_protocol::workflows::WatchSubject;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{PortFailure, PortRefusal};

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
    /// A node is about to send a provider request, and the plugin, a context
    /// source, may add a context block.
    ///
    /// The request carries:
    ///
    /// - `seen`: the text of its own blocks the model receives, oldest first.
    /// - `cwd`: the node's working directory.
    /// - `paths`: how its Host writes paths (`crates/runner-shell/src/lib.rs`).
    /// - `project`: what the plugin's project search last found in the
    ///   conversation's project directory, for a plugin that declares one; none
    ///   until a search answered.
    Context {
        user: UserId,
        conversation: ConversationId,
        node: NodeId,
        cwd: String,
        paths: HostPaths,
        turn: TurnId,
        seen: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<Value>,
    },
    /// The plugin declares a project search: it searches `directory`, the
    /// conversation's project directory, on its primary Host, with the port's
    /// reads, and answers what it found and what tells of a change to it.
    ///
    /// The backend keeps the answer. It searches again when the runner says one
    /// of `watch` changed or the Host comes online again, and gives what it
    /// holds to each context request
    /// (`crates/backend-host-access/src/plugin_files.rs`).
    ProjectSearch {
        user: UserId,
        conversation: ConversationId,
        directory: String,
        paths: HostPaths,
    },
    /// The plugin's page state: the user's, or, with `conversation`, that
    /// conversation's.
    ///
    /// With `held`, the state as it goes along with the conversation's work
    /// panel ([`crate::State::held`]): what the plugin holds, or reads from
    /// its Host once when the panel has tabs of its kind and it holds none;
    /// `Reply::Done` when it has neither.
    PageState {
        user: UserId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        conversation: Option<ConversationId>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        held: bool,
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
    /// The user removed a tab of one of the plugin's panel kinds; the backend applied the change
    /// and answered the page (`crates/backend-http/src/panel.rs`). `tab` is the tab as it was when
    /// it was removed. The user creates no tab of a plugin's kind: the plugin does.
    PanelTab {
        user: UserId,
        conversation: ConversationId,
        tab: PanelTab,
    },
    /// The command package the plugin's commands bind said `message` about
    /// `conversation`, from a service on one of the conversation's Hosts.
    ///
    /// The package says so the moment its state changes and the plugin never
    /// asks whether it did: no job's end, reconnect or timer makes a plugin
    /// read a Host again. What a message says is for the package and its plugin
    /// to agree. The runner and the backend only carry it, and a plugin hears
    /// only the package its commands bind.
    PackageMessage {
        user: UserId,
        conversation: ConversationId,
        message: Value,
    },
    /// What the plugin's command package kept for `conversation` on a Host
    /// ended without the package saying so: its service crashed, its runner
    /// started anew or its Cloud stopped.
    PackageLost {
        user: UserId,
        conversation: ConversationId,
    },
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
    /// A project search's result, and the files and folders whose change
    /// changes it.
    Project {
        found: Value,
        watch: Vec<WatchSubject>,
    },
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
