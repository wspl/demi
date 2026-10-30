//! A job's shell in a process of its own, which outlives the runner that
//! started it: a runner that updates or crashes leaves its jobs running, and
//! the runner that starts next takes them over.
//!
//! ```text
//! runner                                  shell process (one per job)
//! +------------------------------+        +------------------------------+
//! | RemoteShell, RemoteJob       | socket | serve: the job in brush,     |
//! | (the job shell contract)     |<------>| Demi's utilities, programs   |
//! +------------------------------+        +------------------------------+
//!          ^                                  |
//!          +-- local endpoint <-- declared ---+
//!              (`forward.rs`)     commands
//! ```
//!
//! # The process
//!
//! The runner starts its own executable as `demi-runner shell --job <id>`. The
//! process listens on a private local socket of its own, writes its address on
//! its standard output, and runs one job, which the runner sends it. On Unix
//! it forks first and the process the runner started ends, so the shell is in
//! a session of its own and no child of the runner's, which the system reaps;
//! on Windows it starts outside the runner's job objects. Neither the
//! runner's exit nor a service manager's stop of the runner's process group
//! reaches it. It runs the release that started it until its job and what the
//! job left running have ended.
//!
//! # What it holds for the runner
//!
//! The runner keeps the job's output (`crates/runner-jobs/src/kept_output.rs`).
//! The shell numbers each stream's bytes and holds each chunk until the runner
//! says it kept it ([`wire::ToShell::Kept`]), so a runner that connects again
//! receives what it did not keep, and none twice. While no runner takes them,
//! the chunks it holds grow up to [`HELD_BYTES`]; beyond that the shell stops
//! reading the job's output, which then waits in the programs' pipes, as a
//! terminal's paused output does. It holds the job's exit until a runner has
//! it, and ends once its job's work is gone too.
//!
//! # The contract on the wire
//!
//! Frames on the socket carry the job shell contract
//! (`crates/runner-process/src/job_shell.rs`), [`wire`] says how. A field or a
//! message a side does not know is ignored, so the contract grows without a
//! new version; [`wire::CONTRACT`] changes only with a change an older side
//! would misread.
//!
//! A declared command the job runs goes to the runner's local endpoint, as a
//! command alias's does (`forward.rs`), so the runner's dispatcher stays the
//! only one.

mod client;
mod forward;
mod server;
mod wire;

pub(crate) use server::signal_peer_task;

pub use client::RemoteShell;
pub use server::serve;

/// The most output a shell holds that no runner has kept: enough for the
/// seconds a runner takes to update or start again from a job that prints
/// fast, and a bound on what a job that never stops printing costs the
/// device's memory while no runner reads it.
const HELD_BYTES: usize = 16 * 1024 * 1024;
