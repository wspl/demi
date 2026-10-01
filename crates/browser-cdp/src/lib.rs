//! Raw CDP commands and WebMCP (`crates-and-packages.md` § browser-cdp): the
//! `cdp` commands on a tab's own debugging connections, validated against the
//! pinned protocol, and the tools pages declare (`browser.md` § CDP commands
//! and events, § Capabilities and WebMCP).

mod catalog;
pub mod commands;
mod sessions;
#[cfg(feature = "testing")]
pub mod testing;
pub mod webmcp;

use chromiumoxide::Page;
use demi_browser_driver::operation::Result;
use demi_command_package_browser_protocol::browser as protocol;

use crate::protocol::Capability;

/// What the WebMCP and CDP command families offer in `page` (`browser.md` §
/// Capabilities and WebMCP).
pub async fn capabilities(page: &Page) -> Result<Vec<Capability>> {
    Ok(vec![webmcp::capability(page).await?, commands::capability()])
}
