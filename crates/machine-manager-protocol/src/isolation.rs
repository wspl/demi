//! How a manager isolates its Clouds: the runtimes it offers, what each
//! enforces, and where a device's storage is.
//!
//! The master chooses one isolation for every Cloud; the backend names it to
//! the manager as it names the private network, on each connection and after
//! each change. A device's storage is on one runtime, which the manager
//! moves it from to the chosen one while the device is stopped.

use serde::{Deserialize, Serialize};

/// How a Cloud is isolated from the host and from the other Clouds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
    /// A directory tree under proot, which needs no root and protects
    /// against mistakes only.
    Simple,
    /// A gVisor sandbox over ext4 images.
    Gvisor,
}

impl Isolation {
    pub const ALL: [Isolation; 2] = [Isolation::Simple, Isolation::Gvisor];
}

/// What a manager can do with one isolation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeOffer {
    pub isolation: Isolation,
    /// What the runtime enforces, or why this manager cannot run it.
    pub offer: Result<Enforces, Unavailable>,
}

/// What a runtime enforces of the settings the backend shows: the backend
/// asks for none a runtime lacks, and shows each such setting disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Enforces {
    /// A running Cloud's storage is published while its processes run.
    pub checkpoints: bool,
    /// Each user's Cloud Storage bounds the Cloud's filesystems.
    pub storage_limits: bool,
    /// Private Network Access decides what Clouds reach.
    pub private_network: bool,
}

/// Why a manager cannot run a runtime, such as a missing program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Unavailable {
    pub reason: String,
}

/// A device's move to the isolation the master chose, which is the one its
/// storage is not on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum MoveStatus {
    /// The manager copies the device's storage.
    Moving,
    /// The last move failed, and the device stays where it was until its
    /// next wake tries again.
    Failed { message: String },
}
