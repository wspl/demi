//! Instance settings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Who configures providers, fixed for the instance's lifetime
/// (`product.md` § Instance mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstanceMode {
    Shared,
    Isolated,
}

serde_plain::derive_display_from_serialize!(InstanceMode);
serde_plain::derive_fromstr_from_deserialize!(InstanceMode);
