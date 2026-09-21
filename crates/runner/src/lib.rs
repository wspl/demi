//! Native runner: machine IO, jobs, command dispatch and backend connection.

pub mod files;
pub mod fs;
pub mod git;
pub mod paths;
pub mod pipes;
pub mod process;
pub mod state;

pub mod connection;
pub mod host;
pub mod host_log;
pub mod management;
pub mod mode;
pub mod net;
pub mod shell;
pub mod stdio;
pub mod tasks;
pub mod volumes;

pub mod commands;

pub(crate) mod file_diff;
pub(crate) mod tree_watch;
