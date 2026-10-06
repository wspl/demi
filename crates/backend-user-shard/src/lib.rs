//! The user shard (`concurrency.md` § The user shard): the shard threads,
//! each user's shard with the calls into it, and the shared services every
//! shard is given; the conversations as the agent sees them, their idle
//! watches, and the pages' product state and synchronization channels. The shard implements what the Cloud, the exposes, the
//! conversations' host access and the conversation permissions need of it
//! (`CloudShard`, `ExposeShard`, `HostShard`, `PermissionShard`).

pub mod conversation;
#[cfg(feature = "testing")]
pub mod holds;
pub mod lifecycle;
pub mod services;
pub mod shard;
pub mod sync;
pub mod tuning;
