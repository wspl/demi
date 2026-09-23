//! What an invocation carries to a native command: its operation, command
//! context, arguments, working directory and environment, and how it
//! completes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use super::edits::EditContext;

/// The most language tags a [`CommandLocale`] carries.
pub const COMMAND_LOCALE_LANGUAGES: usize = 16;

/// Who started the work: an agent node, or the conversation's user through a
/// user stream. `User` is a struct variant so that unknown fields are refused:
/// serde ignores them for a unit variant of an internally tagged enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum CommandCaller {
    Agent {
        #[garde(length(min = 1))]
        node: String,
    },
    User {},
}

impl CommandCaller {
    pub fn agent(node: impl Into<String>) -> Self {
        Self::Agent { node: node.into() }
    }

    /// The agent node that started the work; none when the user did.
    pub fn node(&self) -> Option<&str> {
        match self {
            Self::Agent { node } => Some(node),
            Self::User {} => None,
        }
    }
}

/// An IANA time zone and BCP 47 language tags in preference order. The
/// browser reports it as a user preference (web-api), so its schema is part
/// of the browser's contract too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandLocale {
    #[garde(length(utf16, min = 1, max = 64))]
    pub time_zone: String,
    #[garde(
        length(min = 1, max = COMMAND_LOCALE_LANGUAGES),
        inner(length(utf16, min = 1, max = 64))
    )]
    pub languages: Vec<String>,
}

/// What a declared command knows beyond its arguments. The backend is its
/// only source; nothing reads it from the environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandContext {
    #[garde(length(min = 1))]
    pub conversation: String,
    #[garde(dive)]
    pub caller: CommandCaller,
    #[garde(dive)]
    pub locale: CommandLocale,
}

/// The metadata that opens a native command invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Invocation {
    #[garde(length(min = 1))]
    pub operation: String,
    #[garde(length(min = 1))]
    pub invocation_id: String,
    #[garde(dive)]
    pub context: CommandContext,
    /// The operation's arguments, a JSON object.
    #[garde(custom(json_object))]
    pub args: serde_json::Value,
    #[garde(length(min = 1), custom(without_nul))]
    pub cwd: String,
    #[garde(custom(environment))]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[garde(dive)]
    pub edits: Option<EditContext>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[garde(skip)]
    pub json: Option<bool>,
}

/// Raw CLI metadata from the local command client (`commands.md` § External
/// command clients). The client names only its opaque execution context, in
/// `args`; the runner finds the command context through it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalInvocation {
    #[garde(length(min = 1))]
    pub operation: String,
    #[garde(length(min = 1))]
    pub invocation_id: String,
    #[garde(custom(json_object))]
    pub args: serde_json::Value,
    #[garde(length(min = 1), custom(without_nul))]
    pub cwd: String,
    #[garde(custom(environment))]
    pub env: BTreeMap<String, String>,
}

/// Why a command failed, in the command's own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

/// How an invocation ended: its exit code, and the error when it failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Completion {
    pub exit_code: u8,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    pub error: Option<CommandError>,
}

/// A garde rule: the value holds no NUL character, as paths and environment
/// values must not.
pub fn without_nul(value: &str, _: &()) -> garde::Result {
    if value.contains('\0') {
        return Err(garde::Error::new("contains a NUL character"));
    }
    Ok(())
}

fn json_object(value: &serde_json::Value, _: &()) -> garde::Result {
    if !value.is_object() {
        return Err(garde::Error::new("is not a JSON object"));
    }
    Ok(())
}

/// Variable names are non-empty and hold neither NUL nor `=`; values hold no
/// NUL.
fn environment(variables: &BTreeMap<String, String>, _: &()) -> garde::Result {
    for (name, value) in variables {
        if name.is_empty() || name.contains(['\0', '=']) {
            return Err(garde::Error::new(format!(
                "invalid environment variable name {name:?}"
            )));
        }
        if value.contains('\0') {
            return Err(garde::Error::new(format!(
                "environment variable {name} contains a NUL character"
            )));
        }
    }
    Ok(())
}
