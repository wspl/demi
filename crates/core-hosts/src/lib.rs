//! A core's Hosts: the runners connected to it (`crates/runner/src/main.rs`,
//! `crates/runner-jobs/src/commands/mod.rs`).
//!
//! The crate holds:
//!
//! - Each device's runner connection, and the Host handles made over it.
//! - The file gate whose lease a conversation's Host is made against.
//! - The routing of a job's rpc calls to its agent node's commands.
//! - The command context work carries.
//! - The file listings and text the product reads from a Host.
//!
//! A Host handle a conversation owns is given out only against that
//! conversation's file-gate lease: `Devices::conversation_host` takes a
//! `FileLease`, which names its conversation, and keys the handle by it. The
//! gates live in the conversations' slots of host access, and only host access
//! takes their leases. Device access and machine access make handles that touch
//! no conversation's files.
//!
//! The file gate is a `shared-gates` gate, which is why the crate depends on
//! `shared-gates`. The crate reaches a conversation's Host only through host
//! access and knows no conversations or Cloud. How a runner is paired, the
//! installers and the releases its runner downloads are the server's
//! (`backend-runners`).

pub mod command_context;
pub mod devices;
pub mod file_gate;
pub mod files;
pub mod host_key;
pub mod packages;
pub mod router;
