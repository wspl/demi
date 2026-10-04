//! Shell jobs and the commands they run (`crates-and-packages.md`
//! § `runner-jobs`): the job table, each job's execution context with its
//! directory, kept output and edit report, the command dispatcher with local
//! command forwarding, and the connection handle the jobs reach the backend
//! through, which the composition's connection owner serves.

pub mod commands;
pub mod connection;
pub mod edit_report;
pub mod job_directories;
pub mod job_media;
pub mod kept_output;
pub mod tasks;
#[cfg(feature = "testing")]
pub mod testing;
