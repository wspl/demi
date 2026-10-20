//! Runners and their devices (`crates/runner/src/main.rs`, `crates/runner-jobs/src/commands/mod.rs`): pairing with its
//! pending claims and codes, the installer scripts, each device's runner
//! connection and the Host handles made over it, the file gate whose lease a
//! conversation's Host is made against, the routing of a job's rpc calls to
//! its agent node's commands, the command context work carries, the file
//! listings and text the product reads from a Host, native package
//! publication with the artifacts sourced on demand and the older releases'
//! packages removed, the next runner release
//! the paired devices download ahead, and the backend's
//! public address that runners reach.

pub mod claims;
pub mod codes;
pub mod command_context;
pub mod devices;
pub mod file_gate;
pub mod files;
pub mod host_key;
pub mod install;
pub mod native;
pub mod prefetch;
pub mod public_url;
pub mod publication;
pub mod retention;
pub mod router;
pub mod sourcing;
