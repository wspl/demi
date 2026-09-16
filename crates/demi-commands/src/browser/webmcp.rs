//! Browser webmcp command family.

use demi_command_service::InvocationContext;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{BrowserEnvironment, BrowserError, BrowserTab, Result, protocol::BrowserCommand};

/// Execute webmcp within the invoking conversation's browser ownership.
pub(super) async fn execute(
    _context: &InvocationContext,
    _environment: &BrowserEnvironment,
    _tab: Option<&BrowserTab>,
    _command: &BrowserCommand,
    _cancel: &CancellationToken,
    _deadline: tokio::time::Instant,
) -> Result<Value> {
    Err(BrowserError::UnsupportedCapability(
        "webmcp implementation is pending the catalog checkpoint".into(),
    ))
}
