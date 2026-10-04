//! The command wire and its SDK at their boundary, in one test binary: the
//! record framing, the media a handler returns, package descriptors, the
//! service protocol, concurrency, the conversation endpoint, the numbers
//! stream and the edit recorder. Only
//! the open-file test keeps a binary of its own (`tests/open_files.rs`): it
//! lowers the process's open-file limit and holds every descriptor left.

mod concurrency;
mod conversation;
mod edits;
mod framing;
mod media;
mod numbers;
mod package;
mod service;
