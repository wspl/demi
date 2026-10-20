//! The plugin host's scenarios: the command set it composes from the `file`
//! plugin, in an agent server whose shells are real runner jobs and whose model
//! is a script.
//!
//! A scenario that takes more than a second (`testing.md` § Cost) says above it
//! which part of its contract takes the time. Measured alone on four cores
//! (2026-09-26):
//!
//! - Every script the model runs is a shell job, whose login shell reads the
//!   machine's profile, about 0.4 s where that loads nvm and rbenv
//!   (`crates/runner-shell/src/lib.rs`).
//! - The first `demi file` on a runner starts the `demi.file` service.
//! - The runner itself starts in a few hundred milliseconds.

mod file;
mod frames;
mod hosts;
mod marathon;
mod outbound;
mod registry;
mod secrets;
mod shell_tools;
mod sign_in;
mod support;
