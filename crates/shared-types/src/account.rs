//! An agent's subscription account under its entry's rotation
//! (`crates/backend-providers/src/llm/rotation.rs`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which account each agent of a subscription entry uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    /// The entry's active account, as without rotation.
    #[default]
    Manual,
    /// The first account in the list's order that is not used up.
    InOrder,
    /// The account with the most room.
    Balanced,
}

serde_plain::derive_display_from_serialize!(Rotation);
serde_plain::derive_fromstr_from_deserialize!(Rotation);

/// The account an agent took, kept with its session's state so a restart
/// keeps it, and the rotation it was taken under: a change of the entry's
/// rotation takes an account again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentAccount {
    #[garde(length(min = 1))]
    pub credential_id: String,
    #[garde(skip)]
    pub rotation: Rotation,
}
