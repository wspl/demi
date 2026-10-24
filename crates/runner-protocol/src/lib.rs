//! The contract between the backend and a runner: the wire's messages, the
//! command manifest, and a managed runner's boot file; and where a Cloud image
//! puts what the runner reads. Both ends link this crate, so each is defined
//! once.
//!
//! - A job's and a resident stream's messages carry the command context.
//! - `conversation_release` is the one generic release of a conversation, which
//!   says whether the conversation is over, archived or deleted,
//!   `package_release` the one of a package's residents for every
//!   conversation, and `numbers_reserve` the one generic request for a
//!   conversation's numbers.
//! - `package_message` and `package_lost` carry a package's word to its plugin
//!   without reading it, so the wire holds no conversation browser policy.
//! - The bytes of the wire are pinned at both ends by a golden corpus with one
//!   message of every kind.
//!
//! The crate holds no network IO, Host implementation, shell environment, job
//! table, credentials, claim policy, device registry or conversation state.

pub mod boot;
pub mod console;
pub mod direct;
pub mod exposure;
pub mod files;
pub mod forward;
pub mod image;
pub mod installation;
pub mod manifest;
pub mod release;
pub mod search;
pub mod values;
pub mod wire;

/// The variable the service manager's registration sets for the runner it
/// starts, which therefore never hands the installation over to the
/// manager (`crates/runner/src/autostart.rs`).
pub const SUPERVISED: &str = "DEMI_SUPERVISED";
