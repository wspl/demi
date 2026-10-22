//! The command wire and its SDK at their boundary, in one test binary.
//!
//! The tests cover:
//!
//! - the record framing and the media a handler returns;
//! - the service protocol's concurrency and flow control;
//! - concurrency and the conversation endpoint;
//! - the numbers and messages streams;
//! - the edit recorder.
//!
//! Only the open-file test keeps a binary of its own (`tests/open_files.rs`):
//! it lowers the process's open-file limit and holds every descriptor left.

mod concurrency;
mod conversation;
mod edits;
mod framing;
mod media;
mod messages;
mod numbers;
mod service;
