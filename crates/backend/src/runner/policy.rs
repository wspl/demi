//! The backend's rules for the calls a runner connection relays
//! (`sessions-and-targets.md` § Bind jobs to their caller): a call runs only
//! for a live job on the connection whose command context names the
//! conversation of the Host that started it, and it reaches that job's agent
//! node's commands and storage.

use std::rc::{Rc, Weak};

use demi_host_remote::{JobOrigin, LinkPolicy};
use demi_runner_protocol::wire::VolumeName;
use demi_shell::{PortError, RpcError, RpcInvocation, RpcPort, StorageOp, StorageReply};
use demi_web_api::ids::DeviceId;
use futures_util::future::LocalBoxFuture;

use super::conversation_of;
use crate::shard::Shard;

/// The rules of one device's connection, in its user's shard.
pub(crate) struct ShardPolicy {
    /// Weak: the shard owns the connection this policy serves.
    shard: Weak<Shard>,
    device: DeviceId,
}

impl ShardPolicy {
    pub(crate) fn new(shard: &Rc<Shard>, device: DeviceId) -> Self {
        Self {
            shard: Rc::downgrade(shard),
            device,
        }
    }

    fn shard(&self) -> Result<Rc<Shard>, String> {
        self.shard.upgrade().ok_or_else(|| "backend shutting down".to_owned())
    }
}

impl LinkPolicy for ShardPolicy {
    fn admit_call(&self, job: &JobOrigin) -> Result<(), String> {
        match conversation_of(&job.host) {
            Some(conversation) if conversation == job.context.conversation => Ok(()),
            Some(_) => Err("rpc job belongs to another conversation".into()),
            None => Err("rpc requires a live job dispatched to this device".into()),
        }
    }

    fn dispatch(
        &self,
        job: Rc<JobOrigin>,
        invocation: RpcInvocation,
        port: RpcPort,
    ) -> LocalBoxFuture<'static, Result<u8, RpcError>> {
        match self.shard() {
            Ok(shard) => shard.commands().dispatch(&job, invocation, port),
            Err(reason) => Box::pin(async move { Err(RpcError::Failed(reason)) }),
        }
    }

    fn storage(&self, job: Rc<JobOrigin>, op: StorageOp) -> LocalBoxFuture<'static, Result<StorageReply, PortError>> {
        match self.shard() {
            Ok(shard) => shard.commands().storage(&job, op),
            Err(reason) => Box::pin(async move { Err(PortError::Ended(reason)) }),
        }
    }

    fn grow_volume(&self, volume: VolumeName, bytes: u64) -> LocalBoxFuture<'static, Result<(), String>> {
        // Only the user's Cloud grows its volumes (`managed-hosts.md` §
        // Lifecycle and capacity), and its machine is not in this backend's
        // shard yet.
        let device = self.device.clone();
        Box::pin(async move {
            tracing::warn!(device = %device, %volume, bytes, "volume growth refused");
            Err("Volume growth is unavailable".into())
        })
    }
}
