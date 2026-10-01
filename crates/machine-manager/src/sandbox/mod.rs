//! One boot of a device's sandbox (`managed-hosts.md` § Container
//! initialization).

#[cfg(target_os = "linux")]
mod boot;
#[cfg(target_os = "linux")]
pub mod cgroup;
pub mod files;
pub mod oci;
pub mod runsc;

#[cfg(target_os = "linux")]
pub use boot::{Capture, Sandbox, SandboxError};
