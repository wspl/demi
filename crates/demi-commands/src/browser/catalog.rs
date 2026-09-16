//! Catalog checkpoint entries completed by the subsequent implementation checkpoint.
use super::{BrowserError, Result};

pub(super) async fn pending() -> Result<serde_json::Value> {
    Err(BrowserError::UnsupportedCapability(
        "implementation is pending the catalog checkpoint".into(),
    ))
}
