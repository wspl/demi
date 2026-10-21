//! The Host operations the backend asks for.
//!
//! The crate answers:
//!
//! - filesystem operations and file contents through pipes;
//! - the working tree with its status and diffs;
//! - the subscriptions to what changes in files, with the numbers that order
//!   reads and changes;
//! - the volumes a Host reports.
//!
//! # Authority
//!
//! The runner performs the operations on the device as the account it runs
//! under, whose own permissions decide which paths they reach. A Host's default
//! working directory is where work starts when a request names none, never a
//! sandbox, a workspace boundary or a permission check.
//!
//! No shell job is involved. When the backend drops the handle of a raw process
//! that has not ended, it asks the runner to kill it and does not wait, since
//! nothing controls it any more.
//!
//! # One request per answer
//!
//! Every request crosses the Host's connection, which may take hundreds of
//! milliseconds each way. An operation the backend needs as one answer is
//! therefore one request, never one per entry. A home directory of 105 entries,
//! listed with one request per entry, took about fifty seconds to show in the
//! folder picker, and takes one round trip so.
//!
//! That is why a listing carries each entry's metadata and a symbolic link's
//! target, and why several paths are looked at, several files read and a
//! directory written with one request each.
//!
//! # Concurrency
//!
//! Host work that finishes on its own runs on blocking threads off the control
//! thread, a few at a time:
//!
//! | Work | Slots |
//! | --- | --- |
//! | Filesystem requests | 32 |
//! | Working-tree requests | 8 |
//! | Computations | 2 |
//! | Syncs | 4 |
//!
//! A request beyond that waits for a slot, still ending when its connection
//! closes.
//!
//! The crate knows no jobs, commands or the connection that carries the
//! requests.

pub mod files;
pub mod fs;
pub mod git;
mod history;
pub mod host;
pub mod numbers;
pub mod subscriptions;
pub mod volumes;
pub mod watches;

mod ignore;
mod tree_watch;
