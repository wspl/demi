//! The coding agent's scenarios: the harness in an agent server whose shells
//! are real runner jobs, and whose model is a script.
//!
//! A scenario that takes more than a second (`testing.md` § Cost) says above
//! it which part of its contract takes the time. Measured alone on four
//! cores (2026-09-26): every script the model runs is a shell job, whose
//! login shell reads the machine's profile, about 0.4 s where that loads nvm
//! and rbenv (`runner.md` § Shell jobs); the first `demi file` on a runner
//! starts the `demi.builtin` service, and the runner itself starts in a few
//! hundred milliseconds.

mod file;
mod frames;
mod hosts;
mod marathon;
mod support;
