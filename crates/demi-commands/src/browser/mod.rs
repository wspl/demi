//! Native browser primitives. Command exposure requires trusted retained-resource scope.

mod actions;
mod element;
mod environment;
mod evaluation;
mod handles;
mod installation;
mod keyboard;
mod navigation;
mod observation;
mod operation;
mod output;
mod protocol;
mod resources;
mod select;
mod tab;

pub use environment::{BrowserEnvironment, LaunchOptions, with_browser};
pub use operation::{BrowserError, Result};
pub(crate) use protocol::OPERATIONS;
pub(crate) use resources::Resources;
pub use tab::BrowserTab;
