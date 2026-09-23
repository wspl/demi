//! The private, temporary credential file supplied to a managed runner
//! (`managed-hosts.md` § Managed boot credential).

use serde::{Deserialize, Serialize};

use crate::values::{BackendUrl, DeviceToken};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedBoot {
    pub backend_url: BackendUrl,
    pub device_token: DeviceToken,
}

/// A boot file that is not a valid boot record.
#[derive(Debug, thiserror::Error)]
#[error("invalid managed boot file: {0}")]
pub struct BootError(#[from] serde_json::Error);

impl ManagedBoot {
    /// Decodes a boot file's JSON; its values are checked as they are read.
    pub fn decode(bytes: &[u8]) -> Result<Self, BootError> {
        Ok(serde_json::from_slice(bytes)?)
    }
}
