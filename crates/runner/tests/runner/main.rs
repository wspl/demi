//! The runner's integration tests that can share a process. Each test binary
//! costs a link and, on macOS, a first-launch check when it is new, so the
//! tests live here. The few that change or saturate process-wide state keep
//! binaries of their own: the open file limit, the global subscriber, the
//! utilities' environment, and the load tests, which fill the process's one
//! shell runtime that every test here shares.

mod artifact_cache;
mod command_client;
mod connection;
mod dispatch;
mod edit_tracking;
mod fs;
mod git;
mod host;
mod local;
mod pipes;
mod process;
mod services;
mod shell;
mod tasks;

// Each test's whole-body timeout is a hang guard of 60 s, not a latency
// check: the tests here share one shell runtime and run together, as a
// runner's jobs do, and a tighter bound fails them on a loaded machine.
