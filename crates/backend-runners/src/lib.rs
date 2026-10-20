//! Runners and their devices (`crates/runner/src/main.rs`,
//! `crates/runner-jobs/src/commands/mod.rs`).
//!
//! The crate holds:
//!
//! - Pairing, with its pending claims and codes.
//! - The installer scripts.
//! - Each device's runner connection, and the Host handles made over it.
//! - The file gate whose lease a conversation's Host is made against.
//! - The routing of a job's rpc calls to its agent node's commands.
//! - The command context work carries.
//! - The file listings and text the product reads from a Host.
//! - Native package publication, with the artifacts sourced on demand and the
//!   older releases' packages removed.
//! - The next runner release the paired devices download ahead.
//! - The backend's public address that runners reach.
//!
//! A Host handle a conversation owns is given out only against that
//! conversation's file-gate lease: `Devices::conversation_host` takes a
//! `FileLease`, which names its conversation, and keys the handle by it. The
//! gates live in the conversations' slots of host access, and only host access
//! takes their leases. Device access and machine access make handles that touch
//! no conversation's files.
//!
//! The file gate is a `shared-gates` gate and native publication uploads to the
//! object store, which is why the crate depends on `shared-gates` and
//! `backend-blobs`. The crate reaches a conversation's Host only through host
//! access and knows no conversations or Cloud.

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
