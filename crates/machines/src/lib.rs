//! The Cloud machine manager (`managed-hosts.md`): it runs users' Cloud
//! machines as gVisor sandboxes and serves the backend over a Unix socket.
//! The manager runs only on Linux; elsewhere the crate builds its portable
//! modules for their tests, without the Linux callers that use them.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

pub mod blocking;
pub mod config;
pub mod fault;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub mod lock;
pub mod manager;
#[cfg(target_os = "linux")]
pub mod namespace;
pub mod network;
#[cfg(target_os = "linux")]
pub mod preflight;
#[cfg(target_os = "linux")]
pub mod recovery;
pub mod sandbox;
pub mod server;
pub mod storage;
#[cfg(all(target_os = "linux", any(test, feature = "testing")))]
pub mod testing;
pub mod tools;
