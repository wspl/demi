//! API scenarios against `Backend::start`, on port 0 and a temporary data
//! directory each.
//!
//! A scenario that takes more than a second (`testing.md` § Cost) says above
//! it which part of its contract takes the time. Measured on four cores
//! (2026-09-26): a runner's first native command installs its package from
//! the backend, which downloads and verifies the debug `demi-file` or
//! `demi-browser`, about a second for the browser's, on each runner and again
//! after a Cloud's reset; every shell job starts a login shell, which reads
//! the machine's profile, 0.3 to 0.5 s where that loads nvm and rbenv;
//! pairing a device or booting a Cloud starts a runner, a few hundred
//! milliseconds; and a configured window passes in real time. The first
//! scenario of a process with a package also computes the package's digest
//! and publishes it, about a second for `demi-browser`, which every later one
//! reuses (`support::Built`).

mod accounts;
mod agent_profiles;
mod attachments;
mod auth;
mod audio;
mod automations;
mod blobs;
mod browser;
mod builtin_families;
mod bytedance;
mod minimax;
mod claude;
mod claude_code;
mod cloud;
mod conversations;
mod credentials;
mod deletion;
mod diagnostics;
mod direct;
mod drafts;
mod edge;
mod editing;
mod exposure;
mod failed_turns;
mod families;
mod files;
mod forks;
mod forwards;
mod handoff;
mod history;
mod history_commands;
mod holding_edge;
mod hosts;
mod images;
mod install;
mod instructions;
mod isolation;
mod lifetime;
mod machines;
mod native;
mod notifications;
mod offline;
mod organize;
mod outputs;
mod panel;
mod passkeys;
mod permissions;
mod plugins;
mod provider_commands;
mod providers;
mod rotation;
mod real_browser;
mod real_cloud;
mod restarts;
mod runners;
mod saved;
mod search;
mod settings;
mod shares;
mod side_questions;
mod sign_in;
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
mod versions;
mod videos;
mod work;
mod workflows;
mod workspaces;
