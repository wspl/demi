//! The backend's rules for the calls a runner connection relays
//! (`sessions-and-targets.md` § Bind jobs to their caller): a call runs only
//! for a live job on the connection whose command context names the
//! conversation of the Host that started it, and it reaches that job's agent
//! node's commands. A native service's conversation numbers come only from a
//! conversation of the user that reaches the device (`native-runtime.md`
//! § Conversation numbers).

use std::rc::{Rc, Weak};

use demi_backend_database::sequences;
use demi_backend_remote_host::{JobOrigin, LinkPolicy};
use demi_command_protocol::ServiceSequence;
use demi_host_interface::{RpcError, RpcInvocation, RpcPort};
use demi_runner_protocol::wire::VolumeName;
use demi_shared_types::Sequence;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;

use demi_backend_runners::host_key::conversation_of;

use super::Shard;
use demi_backend_host_access::host_commands::reachable;

/// The rules of one device's connection, in its user's shard.
pub(crate) struct ShardPolicy {
    /// Weak: the shard owns the connection this policy serves.
    shard: Weak<Shard>,
    device: DeviceId,
}

impl ShardPolicy {
    pub fn new(shard: &Rc<Shard>, device: DeviceId) -> Self {
        Self {
            shard: Rc::downgrade(shard),
            device,
        }
    }

    fn shard(&self) -> Result<Rc<Shard>, String> {
        self.shard
            .upgrade()
            .ok_or_else(|| "backend shutting down".to_owned())
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

    /// Only the user's Cloud grows its volumes (`managed-hosts.md`
    /// § Lifecycle and capacity).
    fn grow_volume(
        &self,
        volume: VolumeName,
        bytes: u64,
    ) -> LocalBoxFuture<'static, Result<(), String>> {
        let shard = self.shard();
        let device = self.device.clone();
        Box::pin(async move {
            let grown = shard?
                .cloud_shard()
                .grow_cloud_volume(&device, volume, bytes)
                .await;
            if let Err(error) = &grown {
                tracing::warn!(device = %device, %volume, bytes, "volume growth refused: {error}");
            }
            grown
        })
    }

    /// Reserves the numbers in the conversation's sequence, for a
    /// conversation of the user that reaches this device as its main Host or
    /// an attached one.
    fn reserve_numbers(
        &self,
        conversation: String,
        sequence: ServiceSequence,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<u64, String>> {
        let shard = self.shard();
        let device = self.device.clone();
        Box::pin(async move {
            let shard = shard?;
            let conversation = ConversationId::try_from(conversation.as_str())
                .map_err(|_| format!("no conversation {conversation}"))?;
            let hosts = reachable(shard.host_shard(), &conversation)
                .await
                .map_err(|error| error.to_string())?;
            if !hosts.iter().any(|host| host.device == device) {
                return Err("the conversation does not reach this device".into());
            }
            let sequence = match sequence {
                ServiceSequence::Tab => Sequence::Tab,
            };
            shard
                .services()
                .conversations
                .db(&conversation)
                .call(move |connection| sequences::reserve(connection, sequence, count))
                .await
                .map_err(|error| error.to_string())
        })
    }
}
