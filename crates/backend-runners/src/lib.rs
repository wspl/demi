//! Runners and their devices (`runner.md`, `commands.md`): pairing with its
//! pending claims and codes, the installer scripts, each device's runner
//! connection and the Host handles made over it, the file gate whose lease a
//! conversation's Host is made against, the routing of a job's rpc calls to
//! its agent node's commands, the command context work carries, the file
//! listings and text the product reads from a Host, native artifact
//! publication with the development store, and the backend's public address
//! that runners reach.

pub mod claims;
pub mod codes;
pub mod command_context;
pub mod devices;
pub mod file_gate;
pub mod files;
pub mod host_key;
pub mod install;
pub mod local_store;
pub mod native;
pub mod public_url;
pub mod publication;
pub mod router;
