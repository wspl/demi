//! The command wire and its SDK at their boundary, in one test binary: the
//! record framing, package descriptors, the service protocol, concurrency and
//! the conversation endpoint. Only the open-file test keeps a binary of its
//! own (`tests/open_files.rs`): it lowers the process's open-file limit and
//! holds every descriptor left.

mod concurrency;
mod conversation;
mod framing;
mod package;
mod service;
