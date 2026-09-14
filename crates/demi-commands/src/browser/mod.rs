//! Native browser primitives. Command exposure requires trusted retained-resource scope.

mod environment;
mod evaluation;
mod operation;
mod tab;

pub use environment::{BrowserEnvironment, LaunchOptions, with_browser};
pub use operation::{BrowserError, Result};
pub use tab::BrowserTab;
