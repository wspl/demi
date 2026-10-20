//! The Cloud machine manager: it runs users' Cloud machines as gVisor
//! sandboxes and serves the backend over a Unix socket. The manager runs only on
//! Linux; elsewhere the crate builds its portable modules for their tests,
//! without the Linux callers that use them.
//!
//! ```text
//! Backend (Linux)
//!     | requests over a restricted Unix socket   (demi-machine-manager-protocol)
//!     v
//! Machine manager (Linux, root, private mount namespace)
//!     | OCI bundle and `runsc` commands
//!     v
//! One user's gVisor/systrap sandbox
//!     init -> runner -> jobs and native services -> Chrome
//!     |
//!     +---- authenticated outbound connection ----> Backend (runner socket)
//! ```
//!
//! The backend owns device identity, authentication, Cloud policy and the
//! durable allocation and reset intent; the manager owns sandbox processes,
//! mounts, networking, resource enforcement and stored generations, and has no
//! users, conversations, transcripts or control database. Every deployment
//! provides Cloud, and its only execution platform is gVisor with systrap:
//! there is no selectable hypervisor, ordinary-container fallback or
//! unsandboxed execution on the backend host, and a missing requirement fails
//! startup rather than falling back. A VPS without KVM suffices, since systrap
//! needs no nested virtualization. Docker and containerd are not dependencies:
//! the manager drives the pinned `runsc` through its OCI interface, with a
//! bundle it generates for each boot over a prepared root, not another image
//! build on each wake.
//!
//! The manager is a trusted, privileged service, since it prepares mounts, loop
//! devices, network namespaces, firewall rules and cgroups; its state and
//! runtime directories are closed to ordinary host users, and no sandbox
//! receives these privileges. It controls the host through the kernel's own
//! interfaces rather than administration tools (`rustix` for mounts, loop
//! devices, freezes, namespaces and locks, netlink for interfaces, addresses and
//! routes, files under `/sys` and `/proc`, `oci-spec` for the bundle), so
//! results and errors are values and no command output is parsed. It runs a
//! program only where no library does the job (`tools`).
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
