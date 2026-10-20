//! Shell jobs and the commands they run: the job table, each job's execution context with its
//! directory, kept output and edit report, the conversations' scratch
//! directories, the command dispatcher with local
//! command forwarding, and the connection handle the jobs reach the backend
//! through, which the composition's connection owner serves.
//!
//! A job's command context is written into every native invocation; the crate
//! implements no conversation browser operation. It knows the shell that runs a
//! job's script only through the job shell contract, and does not own the
//! backend connection.

pub mod commands;
pub mod connection;
pub mod edit_report;
pub mod job_directories;
pub mod job_media;
pub mod kept_output;
pub mod scratch;
pub mod tasks;
#[cfg(feature = "testing")]
pub mod testing;
