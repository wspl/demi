//! Native browser primitives. Command exposure requires trusted retained-resource scope.

mod actions;
mod assets;
mod catalog;
mod cdp;
mod clipboard;
mod download;
mod element;
mod environment;
mod evaluation;
mod fetch;
mod handles;
mod installation;
mod keyboard;
mod navigation;
mod observation;
mod operation;
mod output;
mod process;
mod protocol;
mod resources;
mod select;
mod tab;
mod upload;
mod webmcp;

pub use environment::{BrowserEnvironment, LaunchOptions, with_browser};
pub use operation::{BrowserError, Result};
pub(crate) use protocol::OPERATIONS;
pub(crate) use resources::Resources;
pub use tab::BrowserTab;
