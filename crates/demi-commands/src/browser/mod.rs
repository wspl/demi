//! Native browser primitives. Conversation state belongs to this service instance.

mod actions;
mod assets;
mod cdp;
mod clipboard;
mod content;
mod conversations;
mod dialog;
mod download;
mod element;
mod environment;
mod evaluation;
mod fetch;
mod frames;
mod handles;
mod history;
mod installation;
mod keyboard;
mod launch;
mod live;
mod logs;
mod navigation;
mod observation;
mod operation;
mod output;
mod pointer;
mod probe;
mod process;
mod protocol;
mod query;
mod registry;
mod screenshot;
mod select;
mod selection;
mod tab;
mod text;
mod upload;
mod viewport;
mod webmcp;

pub(crate) use conversations::Conversations;
pub use environment::{
    BrowserEnvironment, LaunchOptions, profile_base, sweep_orphans, with_browser,
};
pub use installation::{BrowserDirectories, pinned_archive};
#[cfg(feature = "testing")]
pub use launch::CAPTURE_EXTENSION_ID;
pub use operation::{BrowserError, Result};
pub use tab::BrowserTab;
