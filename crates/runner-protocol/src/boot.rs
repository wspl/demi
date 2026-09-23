//! The private, temporary credential file supplied to a managed runner
//! (`managed-hosts.md` § Managed boot credential).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedBoot {
    #[garde(custom(http_url))]
    pub backend_url: String,
    #[garde(length(utf16, min = 1, max = 4096), custom(without_whitespace))]
    pub device_token: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("invalid managed boot file: {0}")]
    Invalid(String),
}

impl ManagedBoot {
    /// Decodes and validates a boot file's JSON.
    pub fn decode(bytes: &[u8]) -> Result<Self, BootError> {
        let boot: Self = serde_json::from_slice(bytes)?;
        garde::Validate::validate(&boot).map_err(|report| BootError::Invalid(report.to_string()))?;
        Ok(boot)
    }
}

/// `http://` or `https://`, then at least one character and no whitespace.
fn http_url(value: &str, context: &()) -> garde::Result {
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .ok_or_else(|| garde::Error::new("is not an http or https URL"))?;
    if rest.is_empty() {
        return Err(garde::Error::new("names no host"));
    }
    without_whitespace(rest, context)
}

fn without_whitespace(value: &str, _: &()) -> garde::Result {
    if value.chars().any(char::is_whitespace) {
        return Err(garde::Error::new("contains whitespace"));
    }
    Ok(())
}
