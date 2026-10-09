//! What the commands do to a page (`crates-and-packages.md`
//! § command-package-browser-chrome, `page`):
//! element location and state, evaluation, observation, queries and probes,
//! content and screenshots, keyboard, pointer, selection and select options,
//! the actions that combine them, the clipboard, uploads, downloads, fetches
//! and assets, each a function over a tab.

use demi_command_package_browser_protocol::browser as protocol;

pub mod actions;
pub mod assets;
pub mod clipboard;
mod content;
pub mod download;
mod element;
pub mod evaluation;
mod script;
pub mod fetch;
pub mod keyboard;
mod observation;
mod pointer;
pub mod probe;
mod query;
pub mod screenshot;
mod select;
mod selection;
#[cfg(feature = "testing")]
pub mod testing;
pub mod upload;
