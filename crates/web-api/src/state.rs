//! The product snapshot the page loads and revalidates (`web-api.md` §
//! Sidebar mutations, read state and page synchronization).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::auth::UserDto;
use crate::settings::{InstanceMode, Preferences};

/// `GET /state`: the signed-in user, the instance mode and the user's
/// preferences. The backend assembles it on each request, without waking a
/// Cloud or running inference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProductState {
    pub user: UserDto,
    pub mode: InstanceMode,
    pub preferences: Preferences,
}
