//! The Host operations the backend asks for (`runner.md` § Host operations):
//! filesystem operations and file contents through pipes, the working tree
//! with its status, diffs and change watch, the subscriptions to what
//! changes in files with the numbers that order reads and changes, and the
//! volumes a Host reports.

pub mod files;
pub mod fs;
pub mod git;
pub mod host;
pub mod numbers;
pub mod subscriptions;
pub mod volumes;
pub mod watches;

mod ignore;
mod tree_watch;
