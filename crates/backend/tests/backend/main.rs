//! API scenarios against `Backend::start`, on port 0 and a temporary data
//! directory each.
//!
//! A scenario that takes more than a second (`testing.md` § Cost) says
//! above it which part of its contract takes the time. Measured on four
//! cores (2026-09-26): a runner's first native command installs its package
//! from the backend, which downloads and verifies the debug `demi-file` or
//! `demi-browser`, about a second for the browser's, on each runner and again
//! after a Cloud's reset;
//! every shell job starts a login shell, which reads the machine's profile,
//! 0.3 to 0.5 s where that loads nvm and rbenv; pairing a device or booting a
//! Cloud starts a runner, a few hundred milliseconds; and a configured window
//! passes in real time (`scenarios.md` § System under test). The first
//! scenario of a process with a package also computes the package's digest
//! and publishes it, about a second for `demi-browser`, which every later one
//! reuses (`support::Built`).

mod accounts;
mod attachments;
mod auth;
mod blobs;
mod browser;
mod builtin_families;
mod claude;
mod claude_code;
mod cloud;
mod conversations;
mod deletion;
mod direct;
mod drafts;
mod edge;
mod editing;
mod families;
mod files;
mod forks;
mod holding_edge;
mod hosts;
mod install;
mod isolation;
mod machines;
mod native;
mod outputs;
mod panel;
mod permissions;
mod plugins;
mod preview;
mod providers;
mod real_browser;
mod real_cloud;
mod runners;
mod search;
mod settings;
mod skills;
mod startup;
mod streams;
mod subagents;
mod support;
mod sync;
mod titles;
mod uploads;
mod usage;
mod users;
mod work;
mod workspaces;
