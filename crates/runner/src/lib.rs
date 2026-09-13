//! Native runner: machine IO, jobs, command dispatch and backend connection.

pub mod fs;
pub mod git;
pub mod paths;
pub mod pipes;
pub mod process;
pub mod state;

pub mod connection;
pub mod host;
pub mod init;
pub mod management;
pub mod mode;
pub mod shell;
pub mod stdio;
pub mod tasks;
pub mod volumes;

pub mod commands;

pub(crate) mod file_diff;
