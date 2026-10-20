//! The contract between the backend and a runner: the wire's messages, the
//! command manifest, and a managed runner's boot file; and where a Cloud image
//! puts what the runner reads. Both ends link this crate, so each is defined
//! once.
//!
//! - A job's and a service stream's messages carry the command context.
//! - `conversation_release` is the one generic release message, and
//!   `numbers_reserve` the one generic request for a conversation's numbers.
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
pub mod manifest;
pub mod release;
pub mod values;
pub mod wire;

/// The variable that names, to a runner built for tests, the program that
/// stands in for each of the system's service-manager commands, given the
/// command's name as its first argument (`crates/runner/src/autostart.rs`). The test harnesses set it, so that no test's runner
/// registers with the machine's own manager.
pub const SERVICE_MANAGER_STAND_IN: &str = "DEMI_SERVICE_MANAGER_STAND_IN";

/// The variable the service manager's registration sets for the runner it
/// starts, which therefore never hands the installation over to the
/// manager (`crates/runner/src/autostart.rs`).
pub const SUPERVISED: &str = "DEMI_SUPERVISED";
