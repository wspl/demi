//! A program's process's own routes: `POST /cores` opens a core in a
//! directory, `DELETE /cores/{id}` closes it
//! (`crates/core-api/src/process/mod.rs`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::UserId;

/// `POST /cores`: the directory of the core to open, made when it is absent
/// or empty; a relative one is taken from the process's working directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenCore {
    #[garde(length(min = 1))]
    pub dir: String,
}

/// The core opened: its id, under which its routes are at
/// `/cores/{id}/api`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenedCore {
    pub id: UserId,
}
