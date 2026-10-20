//! One boot of a device's sandbox.
//!
//! The OCI process is a shipped minimal init (`tini`) running as `demi`, UID
//! and GID 1000, which starts the runner and reaps orphaned descendants. A
//! runner exit ends the sandbox, and init does not hide repeated crashes by
//! restarting it.
//!
//! The manager supplies the mounts, the network configuration and, with the
//! resource limits on, the cgroup limits before the start. The runner is an
//! ordinary process: it mounts no root filesystem, configures no host
//! networking, parses no kernel boot arguments and infers no boot mode from PID
//! 1.
//!
//! A new home is initialized from the base's `/etc/skel`, and the
//! initialization is recorded with the publication of the storage, so an empty
//! directory never repeats it. An existing home's ownership is never changed
//! recursively at a wake or a reset.
//!
//! Demi's own programs follow the server's release, though a Cloud's system
//! stays on the base it is pinned to: at every boot the manager bind-mounts the
//! configured base's `/opt/demi`, which holds the runner, its `demi` alias and
//! the embedded command programs, read-only over the pinned base's own.
//!
//! For example, a Cloud made on 0.1.0's base has packages its user installed.
//! The server moves to 0.2.0 and the manager imports its image. At the Cloud's
//! next wake its root is still 0.1.0's base under the user's system layer, with
//! the user's packages, while `/opt/demi`, and so the runner that connects to
//! the 0.2.0 backend, is 0.2.0's.
//!
//! A newer base cannot go under the system layer, which holds the user's
//! changes, the package database among them, as files over the base: a new base
//! beneath it would leave a package database that matches neither. A base that
//! a release stops supporting is replaced at the next wake by a reset to the
//! configured base, keeping the home.
//!
//! To stop a sandbox the manager does these steps:
//!
//! 1. It asks every sandbox process to terminate and waits a few seconds.
//! 2. It deletes the runtime.
//! 3. It kills whatever remains in the sandbox's cgroup.
//! 4. It waits for every Sentry and Gofer process to exit before it releases
//!    the storage, so orphaned Chrome, native services and helpers cannot
//!    survive a stop.
//!
//! With the limits off the sandbox has no cgroup and deleting the runtime is
//! the whole fence: `runsc` kills the Sentry, whose PID namespace ends every
//! process of the sandbox, and the Gofer. A runtime that `runsc` was killed
//! while creating, before it recorded that runtime, is then invisible to a
//! later stop or recovery, where the cgroup kill finds it. Under systemd the
//! service's own cgroup still ends such a leftover when the service stops.

#[cfg(target_os = "linux")]
mod boot;
#[cfg(target_os = "linux")]
pub mod cgroup;
pub mod files;
pub mod oci;
pub mod runsc;

#[cfg(target_os = "linux")]
pub use boot::{Capture, Sandbox, SandboxError};
