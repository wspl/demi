//! The Host operations the backend asks for (`runner.md` § Host operations):
//! filesystem operations and file contents through pipes, the working tree
//! with its status, diffs and change watch, network streams, and the volumes
//! a Host reports.

pub mod files;
pub mod fs;
pub mod git;
pub mod host;
pub mod net;
pub mod volumes;

mod tree_watch;
