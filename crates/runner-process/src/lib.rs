//! The runner's child processes and their IO.
//!
//! The crate holds:
//!
//! - starting and reaping processes, and standard IO;
//! - the way to the backend and its pipes;
//! - lines and tails of streams;
//! - private state files, and line counts of a change;
//! - the steps that may hold up the control thread, and its watch;
//! - private local sockets, the local command client, and the job shell
//!   contract.
//!
//! The crate knows nothing of the backend connection, jobs, commands, residents
//! or the shell that implements the job shell contract.

pub mod backend;
pub mod command_client;
#[cfg(windows)]
pub mod detached;
pub mod job_shell;
pub mod lines;
pub mod local_socket;
pub mod pipes;
pub mod private_files;
pub mod process;
pub mod stdio;
pub mod steps;
pub mod tail;
