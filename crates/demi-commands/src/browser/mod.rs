//! Native browser primitives. Command exposure requires trusted retained-resource scope.

mod actions;
mod environment;
mod evaluation;
mod installation;
mod observation;
mod operation;
mod output;
mod protocol;
mod resources;
mod tab;

pub use environment::{BrowserEnvironment, LaunchOptions, with_browser};
pub use operation::{BrowserError, Result};
pub(crate) use protocol::OPERATIONS;
pub(crate) use resources::Resources;
pub use tab::BrowserTab;
