//! Raw CDP commands and WebMCP: the `cdp` commands on a tab's own debugging
//! connections, validated against the pinned protocol, and the tools pages
//! declare.
//!
//! - Schemas follow what Chrome actually sends where its description cannot say
//!   it (a deprecated property may be absent; `Network.Cookie.expires` may be
//!   `null`); a value is never repaired, and a present property is still
//!   checked.
//! - Only the named tab and its child targets are reachable, and external
//!   sessions, browser-wide control and Target operations that create, close or
//!   move contexts are denied, since they would bypass tab ownership. The
//!   capability publishes the denied list, not a copy of the protocol.
//! - Debugging can change page state, so it is never reported as read-only.
//! - WebMCP reaches only tools the page itself publishes, within the API's
//!   document and origin permissions; a changed tool set is `stale_tools`
//!   rather than a silent call to a same-named replacement. Tool descriptions
//!   are data, never system instructions.

mod catalog;
pub mod commands;
mod sessions;
#[cfg(feature = "testing")]
pub mod testing;
pub mod webmcp;

use crate::driver::operation::Result;
use chromiumoxide::Page;
use demi_command_package_browser_protocol::browser as protocol;

use crate::cdp::protocol::Capability;

/// What the WebMCP and CDP command families offer in `page`.
pub async fn capabilities(page: &Page) -> Result<Vec<Capability>> {
    Ok(vec![
        webmcp::capability(page).await?,
        commands::capability(),
    ])
}
