//! Conversations as the browser creates, lists and follows them
//! (`web-api.md` § Conversation creation and Fork, § Sidebar mutations, read
//! state and page synchronization).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::ids::{DeviceId, WorkspaceId};

/// Where a conversation's work runs (`sessions-and-targets.md` § Resolve a
/// target): the user's Cloud, a directory on a paired device, or a
/// workspace. A new conversation runs on the Cloud, in its session
/// directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ConversationTarget {
    Cloud {
        /// An absolute directory on the Cloud; the conversation's session
        /// directory without it.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(inner(length(min = 1), pattern(r"^/")))]
        path: Option<String>,
    },
    Device {
        #[garde(skip)]
        device_id: DeviceId,
        #[garde(length(min = 1))]
        path: String,
    },
    Workspace {
        #[garde(skip)]
        workspace_id: WorkspaceId,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_target_names_its_kind_and_refuses_what_its_kind_does_not_hold() {
        let cloud: ConversationTarget = demi_core::decode(r#"{"kind":"cloud"}"#).unwrap();
        assert_eq!(cloud, ConversationTarget::Cloud { path: None });
        assert_eq!(serde_json::to_value(&cloud).unwrap(), json!({ "kind": "cloud" }));
        let device = ConversationTarget::Device {
            device_id: DeviceId::try_from("laptop").unwrap(),
            path: "/work".into(),
        };
        assert_eq!(
            serde_json::to_value(&device).unwrap(),
            json!({ "kind": "device", "deviceId": "laptop", "path": "/work" })
        );
        for refused in [
            r#"{"kind":"cloud","path":"relative"}"#,
            r#"{"kind":"cloud","path":null}"#,
            r#"{"kind":"device","deviceId":"laptop","path":""}"#,
            r#"{"kind":"workspace","workspaceId":"w1","path":"/work"}"#,
            r#"{"kind":"elsewhere"}"#,
        ] {
            assert!(demi_core::decode::<ConversationTarget>(refused).is_err(), "{refused}");
        }
    }
}
