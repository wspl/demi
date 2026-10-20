//! The conversation browser's Chrome, in five layers, each a module that knows
//! only the ones beneath it:
//!
//! ```text
//! live    viewers of the browser, capture of the tabs they watch, input
//! cdp     raw CDP commands and WebMCP
//! page    what the commands do to a page
//! tabs    each environment, its tab registry, each tab's state and lock
//! driver  Chrome, its process and profile, the operation every command runs as
//! ```
//!
//! The `demi.browser` program (`command-package-browser`) composes them.
//!
//! - chromiumoxide (vendored) integrates the process, CDP calls and events;
//!   Demi owns targeting, actionability, input ownership and retirement. A
//!   library helper is used only when it matches the command contract: a
//!   successful input dispatch does not prove the control was enabled or
//!   unobstructed.
//! - Event subscriptions are bounded and report what they lose; a slow listener
//!   never stops control requests. Lost registry events are reconciled before
//!   the next operation.
//! - One observation and targeting implementation serves every command; there
//!   is no parallel accessibility, DOM and coordinate state.
//! - A CDP message over 64 MiB is discarded as it is read, so it never costs
//!   more than one read. A response fails only its request
//!   (`result_too_large`); an event cannot be dropped without leaving
//!   observations stale, so it fails the connection into the normal
//!   browser-loss cleanup.

pub mod cdp;
pub mod driver;
pub mod live;
pub mod page;
pub mod tabs;
