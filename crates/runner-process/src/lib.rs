//! The runner's child processes and their IO (`crates-and-packages.md`
//! § `runner-process`): starting and reaping processes, standard IO, the way
//! to the backend and its pipes, lines and tails of streams, private state files, line
//! counts of a change, the local command client, and the job shell contract.

pub mod backend;
pub mod command_client;
pub mod file_diff;
pub mod job_shell;
pub mod lines;
pub mod pipes;
pub mod private_files;
pub mod process;
pub mod stdio;
pub mod tail;
