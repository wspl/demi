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
//!
//! ```text
//! loop thread: LocalRuntime; owns the manager state
//!   the socket server and its connections; one task per request
//!   one DeviceWorker per device: owns its sandbox, runs its operations in order
//!   Admission: a fair semaphore; device operations share it,
//!     reconcile and shutdown take it whole
//! blocking pool, through one entry point: file work and fsync, mounts, loop and
//!   freeze ioctls, sparse copies, hashing, firewall updates
//! one-shot threads: work inside another namespace; each exits after one job
//! child processes: runsc, mke2fs, e2fsck, resize2fs, bsdtar, nft
//! ```
//!
//! Namespace work never runs on a pool thread: one that entered another
//! namespace would run later jobs in the wrong one. Recovery of the saved mount
//! namespace is a separate process, which the manager starts by re-executing
//! itself through `/proc/self/exe` and which enters the namespace before
//! `exec`, so every thread and child of that process lives in it. The manager
//! never cancels a device operation: shutdown drains the device queues, and a
//! checkpoint's frozen window is one synchronous job whose guard always thaws
//! the filesystem. The manager listens on no TCP socket, links no backend and
//! executes no image content on the host; workload credentials use
//! `runner-protocol`'s `ManagedBoot`. The crate also holds the Lima
//! configuration and the pinned `runsc` build inputs.
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
