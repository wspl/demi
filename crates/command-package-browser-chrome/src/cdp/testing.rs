//! What only the Chrome tests reach: a target no command addresses.

use chromiumoxide::cdp::browser_protocol::target::{CloseTargetParams, TargetId};

use crate::driver::operation::Result;
use crate::tabs::environment::BrowserEnvironment;

/// Closes `target`, for the Chrome tests: the capture extension's document,
/// as Chrome may end it.
pub async fn close(environment: &BrowserEnvironment, target: TargetId) -> Result<()> {
    environment
        .browser()
        .call()?
        .execute(CloseTargetParams::new(target))
        .await?;
    Ok(())
}
