//! A conversation browser's environment and its tabs (`crates-and-packages.md`
//! § browser-tabs): the tab registry, each tab's state and operation lock, the
//! data each command family keeps for a tab, viewports, dialogs, console logs
//! and navigation. What acts on a page is written above this crate.

use demi_browser_protocol::browser as protocol;

pub mod debug;
pub mod dialog;
pub mod environment;
pub mod logs;
pub mod navigation;
pub mod registry;
pub mod session;
pub mod tab;
pub mod viewport;
