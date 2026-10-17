//! The contract between the backend and a runner: the wire's messages, the
//! command manifest, and a managed runner's boot file; and where a Cloud
//! image puts what the runner reads. Both ends link this crate, so each is
//! defined once.

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
/// command's name as its first argument (`runner.md` § Starting with the
/// computer). The test harnesses set it, so that no test's runner
/// registers with the machine's own manager.
pub const SERVICE_MANAGER_STAND_IN: &str = "DEMI_SERVICE_MANAGER_STAND_IN";

/// The variable the service manager's registration sets for the runner it
/// starts, which therefore never hands the installation over to the
/// manager (`runner.md` § Starting with the computer).
pub const SUPERVISED: &str = "DEMI_SUPERVISED";
