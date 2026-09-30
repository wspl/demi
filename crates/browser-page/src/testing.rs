//! What only the Chrome tests reach: driving a page by CSS selector.

use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::input::MouseButton;
use tokio_util::sync::CancellationToken;

use demi_browser_driver::operation::{BrowserError, Result};
use demi_browser_tabs::tab::BrowserTab;

use crate::{element, protocol::BrowserTarget};

/// The element target of a CSS selector.
fn css_target(selector: &str) -> BrowserTarget {
    BrowserTarget {
        css: Some(selector.to_owned()),
        ..BrowserTarget::default()
    }
}

/// Clicks the element `selector` matches, for tests that drive a page.
pub async fn click_css(
    tab: &BrowserTab,
    selector: &str,
    cancellation: &CancellationToken,
    timeout: Duration,
) -> Result<()> {
    let mut session = tab.state().gate.try_checkout().ok_or(BrowserError::Busy)?;
    let operation = tab.operation(cancellation, tokio::time::Instant::now() + timeout);
    let target = css_target(selector);
    let (_, state) = crate::element::ready(
        tab,
        &target,
        &mut session.references,
        element::CLICK,
        &operation,
    )
    .await?;
    crate::pointer::click_at(tab, state.point(), MouseButton::Left, 1, 0, &operation).await
}

/// Fills the element `selector` matches, for tests that drive a page.
pub async fn fill_css(
    tab: &BrowserTab,
    selector: &str,
    text: &str,
    cancellation: &CancellationToken,
    timeout: Duration,
) -> Result<()> {
    let mut session = tab.state().gate.try_checkout().ok_or(BrowserError::Busy)?;
    let operation = tab.operation(cancellation, tokio::time::Instant::now() + timeout);
    let target = css_target(selector);
    crate::keyboard::fill(tab, &target, &mut session.references, text, &operation).await
}
