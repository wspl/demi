//! The web plugin through the JSON loopback over an in-memory Demi, whose
//! outbound requests a fake Exa answers: `demi web search` each way, and
//! the Settings › Web Search page's state and methods.

mod exa;
mod search;
mod settings;
mod support;
