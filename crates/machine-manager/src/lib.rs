//! The Cloud machine manager: it runs users' Cloud machines, as gVisor
//! sandboxes or under Simple Isolation (`runtime`), and serves the backend
//! over a Unix socket.
//!
//! The manager runs only on Linux. Elsewhere the crate builds its portable
//! modules, without the Linux callers that use them.
//!
//! ```text
//! Backend (Linux)
//!     | requests over a restricted Unix socket   (demi-machine-manager-protocol)
//!     v
//! Machine manager (Linux; root and a private mount namespace for gVisor)
//!     | OCI bundle and `runsc` commands     | `proot` over a directory tree
//!     v                                     v
//! One user's gVisor/systrap sandbox     One user's Simple Isolation Cloud
//!     init -> runner -> jobs and residents -> Chrome
//!     |
//!     +---- authenticated outbound connection ----> Backend (runner socket)
//! ```
//!
//! The manager's `tracing` events go to the systemd journal, and the events of
//! `info` and above also go to the backend as lines of its log, on each
//! connection it has open (`server`, `crates/shared-types/src/log_layer.rs`).
//!
//! The backend owns device identity, authentication, Cloud policy and the
//! durable allocation and reset intent. The manager owns sandbox processes,
//! mounts, networking, resource enforcement and stored generations, and has no
//! users, conversations, transcripts or control database.
//!
//! The execution platforms:
//!
//! - Every deployment provides Cloud, on gVisor with systrap where the manager
//!   can run it, and on Simple Isolation everywhere else on Linux.
//! - gVisor needs root, a private mount namespace, its programs, the pinned
//!   `runsc` and, with the limits on, the cgroup v2 controllers. A manager
//!   that lacks one says which, and runs Simple Isolation alone; a device
//!   whose storage is on gVisor then fails to start with that reason.
//! - A VPS without KVM suffices, since systrap needs no nested virtualization.
//! - Docker and containerd are not dependencies: the manager drives the pinned
//!   `runsc` through its OCI interface, with a bundle it generates for each
//!   boot over a prepared root, not another image build on each wake.
//!
//! For gVisor the manager is a trusted, privileged service, since it prepares
//! mounts, loop devices, network namespaces, firewall rules and cgroups. Its
//! state and runtime directories are closed to ordinary host users, and no
//! sandbox receives these privileges.
//!
//! It controls the host through the kernel's own interfaces rather than
//! administration tools, so results and errors are values and no command output
//! is parsed:
//!
//! - `rustix` for mounts, loop devices, freezes, namespaces and locks.
//! - Netlink for interfaces, addresses and routes.
//! - Files under `/sys` and `/proc`.
//! - `oci-spec` for the bundle.
//!
//! It runs a program only where no library does the job (`tools`).
//!
//! The manager's threads and processes:
//!
//! ```text
//! loop thread: LocalRuntime; owns the manager state
//!   the socket server and its connections; one task per request
//!   one DeviceWorker per device: owns its runtime, runs its operations in order
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
//! `exec`, so every thread and child of that process lives in it.
//!
//! The manager never cancels a device operation but a move between runtimes,
//! which the next manager makes again: shutdown drains the device queues, and
//! a checkpoint's frozen window is one synchronous job whose guard always
//! thaws the filesystem. The shutdown of an upgrade that asks leaves the
//! running sandboxes to the next manager instead, once their operations end
//! (`handover`).
//!
//! The manager listens on no TCP socket, links no backend and executes no image
//! content on the host. Workload credentials use `runner-protocol`'s
//! `ManagedBoot`. The crate also holds the Lima configuration and the pinned
//! `runsc` build inputs.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

pub mod blocking;
pub mod config;
#[cfg(target_os = "linux")]
pub mod handover;
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
#[cfg(target_os = "linux")]
pub mod runtime;
pub mod sandbox;
pub mod server;
pub mod storage;
pub mod tools;
