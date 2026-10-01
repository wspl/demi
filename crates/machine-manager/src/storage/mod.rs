//! A device's storage (`managed-hosts.md` § Images).

pub mod archive;
#[cfg(target_os = "linux")]
pub mod base;
#[cfg(target_os = "linux")]
pub mod clone;
pub mod durable;
pub mod ext4;
#[cfg(target_os = "linux")]
pub mod skeleton;
pub mod store;
pub mod working;
