//! The user's Cloud: its status and its reset (`web-api.md` § Cloud).

use demi_core::{MAX_SAFE_INTEGER, Nullable};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{DeviceId, OperationId};

/// Where the user's Cloud is in its lifecycle (`managed-hosts.md`
/// § Lifecycle and capacity): `unallocated` until its first use makes it,
/// `off` while no sandbox runs, `booting` until its runner connects,
/// `running`, `saving` while it stops, and `resetting` while a reset holds
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CloudState {
    Unallocated,
    Off,
    Booting,
    Running,
    Saving,
    Resetting,
}

/// A reset's phase (`managed-hosts.md` § System reset): it stops the Cloud,
/// saves its disks, rebuilds the system on the base the reset selected and
/// boots, and ends `ready` or `failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResetPhase {
    Stopping,
    Saving,
    Rebuilding,
    Booting,
    Ready,
    Failed,
}

serde_plain::derive_display_from_serialize!(ResetPhase);
serde_plain::derive_fromstr_from_deserialize!(ResetPhase);

/// The Cloud's latest reset: the operation the page named, its phase, and
/// why it failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloudOperation {
    pub id: OperationId,
    pub phase: ResetPhase,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub error: Option<String>,
}

/// The Cloud's device: its identity, which wake, stop and reset keep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudDevice {
    pub id: DeviceId,
    pub name: String,
}

/// The capacities of a Cloud's two writable filesystems in bytes, or the
/// most each may grow to: the capacity, not what is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloudVolumes {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub system_bytes: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub home_bytes: u64,
}

/// `GET /cloud`, and the `cloud` of the product state: the device, its
/// lifecycle state, its latest reset, why its last boot, save or reset
/// failed, the capacities of its filesystems once its first boot made them,
/// and the most they may grow to. Reading it never wakes the Cloud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloudStatus {
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<CloudDevice>")]
    pub device: Option<CloudDevice>,
    pub state: CloudState,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<CloudOperation>")]
    pub operation: Option<CloudOperation>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub error: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<CloudVolumes>")]
    pub volumes: Option<CloudVolumes>,
    pub limits: CloudVolumes,
}

/// `POST /cloud/reset`: the operation the page names the reset by; a retry
/// with the same id is the same reset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudReset {
    #[garde(skip)]
    pub operation_id: OperationId,
}

/// `{ operation }`: the reset a `POST /cloud/reset` admitted, or the one it
/// names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudResetAnswer {
    pub operation: CloudOperation,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_status_travels_with_its_nullable_fields_and_a_reset_names_its_operation() {
        let status = CloudStatus {
            device: None,
            state: CloudState::Unallocated,
            operation: None,
            error: None,
            volumes: None,
            limits: CloudVolumes {
                system_bytes: 16,
                home_bytes: 32,
            },
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(
            json,
            json!({
                "device": null, "state": "unallocated", "operation": null, "error": null, "volumes": null,
                "limits": { "systemBytes": 16, "homeBytes": 32 },
            })
        );
        assert_eq!(serde_json::from_value::<CloudStatus>(json).unwrap(), status);

        let reset: CloudReset =
            demi_core::decode(r#"{"operationId":"0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b"}"#).unwrap();
        assert_eq!(reset.operation_id.as_str(), "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b");
        for refused in [r#"{"operationId":"reset-1"}"#, r#"{"operationId":""}"#, "{}"] {
            assert!(demi_core::decode::<CloudReset>(refused).is_err(), "{refused}");
        }
        let extra = r#"{"operationId":"0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b","base":"x"}"#;
        assert!(demi_core::decode::<CloudReset>(extra).is_err());
    }
}
