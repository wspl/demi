//! The live view (`crates-and-packages.md`
//! § command-package-browser-chrome, `live`, `live-view.md`):
//! viewers of the conversation's browser, the capture of the tabs they watch,
//! its pacing, and the relay of their input to the tab.

use demi_command_package_browser_protocol::browser as protocol;

pub mod hub;
pub mod viewer;

mod commands;
mod downloads;
mod frames;
mod input;
mod observers;
mod rate;
mod stream;
mod uploads;
mod writer;
