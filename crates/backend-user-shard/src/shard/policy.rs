//! The backend's rules for the calls a runner connection relays.
//!
//! # Who may call
//!
//! A call runs only for a live job on the connection whose command context names
//! the conversation of the Host that started it, and it reaches that job's agent
//! node's commands.
//!
//! - A native service's conversation numbers come only from a conversation of
//!   the user that reaches the device
//!   (`crates/command-protocol/src/numbers.rs`).
//! - A direct stream counts as the activity of such a conversation only.
//! - A service's message reaches its plugin only about such a conversation
//!   (`crates/command-protocol/src/messages.rs`).
//!
//! # Registering a job
//!
//! The backend registers a job before sending it. Its live record binds the
//! device, the shell, the invoking Host, the command context's conversation and
//! the agent node. These stay fixed for the job even when another shell starts or
//! another conversation uses the same device: node `a1`'s job on the primary
//! laptop and its job on attached `ci` each call back for `a1`, each with its
//! own invoking Host.
//!
//! # Callbacks
//!
//! A callback supplies only its job reference. The backend checks the
//! authenticated device connection and the live job record, gives the handler the
//! command context from that record and validates the arguments.
//!
//! - An unknown, completed or disconnected job cannot invoke a callback.
//! - A device token never authorizes another user's jobs.
//! - A job that runs on another Host carries its originating job's context, and
//!   its pipes stay bound to the participating device endpoints.
//!
//! The callback is authorized here, where the backend holds the device's runner
//! connection, in the owner's shard, rather than delegated to the generic Host
//! adapter, which knows no user policy.

use std::rc::{Rc, Weak};

use bytes::Bytes;
use demi_backend_database::sequences;
use demi_backend_host_access::host_commands::{pairing_code, reachable, resolve_host};
use demi_backend_host_access::root_of;
use demi_backend_plugins::Lost;
use demi_backend_remote_host::{DirectAdmission, ForwardEvent, JobOrigin, LinkPolicy, PackageEvent};
use demi_backend_runners::host_key::conversation_of;
use demi_backend_database::permissions::AskedHost;
use demi_backend_permissions::Need;
use demi_command_protocol::ServiceSequence;
use demi_host_interface::{BroughtHost, RpcError, RpcInvocation, RpcPort};
use demi_shared_gates::Purpose;
use demi_shared_types::{BlobRef, Sequence};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;

use super::Shard;

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

    /// Runs the call in its node's commands.
    ///
    /// Once the command set checked the call, what it needs is checked against the
    /// conversation's grants and the hosts it may reach before the handler runs.
    /// What it needs is its leaf's category, the host its `bringsHost` argument
    /// brings into the conversation, or the computer `demi host pair` pairs. A call
    /// that lacks it waits for the user's answer.
    fn dispatch(
        &self,
        job: Rc<JobOrigin>,
        invocation: RpcInvocation,
        port: RpcPort,
    ) -> LocalBoxFuture<'static, Result<u8, RpcError>> {
        let shard = self.shard();
        Box::pin(async move {
            let shard = shard.map_err(RpcError::Failed)?;
            let commands = shard.commands().commands_of(&job).map_err(RpcError::Failed)?;
            let path = invocation.path.clone();
            // A usage error, the check's or the handler's, is told with the
            // leaf's usage.
            let run = async {
                let checked = commands.check(&invocation)?;
                let handler = checked.handler.clone();
                // The host is resolved here once: the check asks for that very
                // device, and the handler acts on it without reading the name
                // again, so a rename meanwhile cannot make them differ.
                let brought = match &checked.brings_host {
                    Some(named) => {
                        let conversation = ConversationId::try_from(invocation.context.conversation.as_str())
                            .map_err(|_| RpcError::Failed("the call names no conversation".into()))?;
                        let host = resolve_host(shard.host_shard(), &conversation, named).await?;
                        Some(BroughtHost {
                            device: host.device.to_string(),
                            name: host.name,
                            cloud: host.cloud,
                        })
                    }
                    None => None,
                };
                // A leaf declares a category or brings a host, never both
                // (`command-declarations`).
                let need = match (pairing_code(&invocation), &brought, checked.category) {
                    (Some(code), _, _) => Some(Need::Host(AskedHost::Pairing(code))),
                    (None, Some(brought), _) => Some(Need::Host(AskedHost::Device(
                        DeviceId::try_from(brought.device.clone())
                            .map_err(|_| RpcError::Failed("the resolved host has no id".into()))?,
                    ))),
                    (None, None, Some(category)) => Some(Need::Category(category)),
                    (None, None, None) => None,
                };
                if let Some(need) = need {
                    demi_backend_permissions::check(shard.permission_shard(), &invocation, need, &port).await?;
                }
                let mut invocation = invocation;
                invocation.brought = brought;
                handler.call(invocation, port).await
            };
            run.await.map_err(|error| commands.tell(&path, error))
        })
    }

    /// The blobs of the user's namespace a handler returns as media.
    fn read_blob(&self, blob: BlobRef) -> LocalBoxFuture<'static, Result<Option<Bytes>, String>> {
        let shard = self.shard();
        Box::pin(async move {
            shard?
                .host_shard()
                .blobs()
                .get(&blob)
                .await
                .map_err(|error| error.to_string())
        })
    }

    /// Restores the tree of the conversation the backend recorded the job
    /// for, as a report of the job does, and has the node that ran the job
    /// take it up; why it could not, when it could not.
    fn take_up_job(&self, job: String) -> LocalBoxFuture<'static, Result<(), String>> {
        let shard = self.shard();
        let device = self.device.clone();
        Box::pin(async move {
            let shard = shard?;
            let conversation = shard
                .services()
                .control
                .running_jobs(device)
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .find(|recorded| recorded.sent && recorded.job == job)
                .map(|recorded| recorded.conversation)
                .ok_or("rpc requires a live job dispatched to this device")?;
            // Counted before the open, so the close waits for it before the
            // agent shuts down.
            let _opening = shard.tree_openers().token();
            if shard.is_closing() {
                return Err("the backend is shutting down".into());
            }
            shard.restore_tree(&conversation).await?;
            shard.agent().take_up_job(&root_of(&conversation), &job).await
        })
    }

    /// Revokes the device at its runner's request, as the user's
    /// revocation does: only a paired device.
    fn revoke_device(&self) -> LocalBoxFuture<'static, Result<(), String>> {
        let shard = self.shard();
        let device = self.device.clone();
        Box::pin(async move {
            let shard = shard?;
            let record = shard
                .services()
                .control
                .device(device.clone())
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "the device is gone".to_owned())?;
            if record.kind != DeviceKind::User {
                return Err("only a paired device is revoked".into());
            }
            shard
                .revoke_device(device)
                .await
                .map(drop)
                .map_err(|error| error.to_string())
        })
    }

    /// Reserves the numbers in the conversation's sequence, for a
    /// conversation of the user that reaches this device as its primary Host or
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
            let conversation = reaching(&shard, &conversation, &device).await?;
            let sequence = match sequence {
                ServiceSequence::Tab => Sequence::Tab,
                ServiceSequence::Device => Sequence::Device,
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

    /// Knows a direct stream of a conversation of the user that reaches this
    /// device as an open user stream.
    ///
    /// It holds the conversation active with a demand lease of its stream gate,
    /// and its idle watch started, as a Host admission starts it
    /// (`crates/backend-user-shard/src/lifecycle/mod.rs`). It installs what a
    /// relay stream of the user stream `name` may: the executable of the package
    /// the name binds, which the runner fetches from where the published catalog
    /// says.
    fn direct_stream(
        &self,
        conversation: String,
        name: String,
    ) -> LocalBoxFuture<'static, Result<DirectAdmission, String>> {
        let shard = self.shard();
        let device = self.device.clone();
        Box::pin(async move {
            let shard = shard?;
            let conversation = reaching(&shard, &conversation, &device).await?;
            let lease = shard
                .conversations()
                .slot(&conversation)
                .streams
                .enter(Purpose::Demand)
                .await;
            shard.track_idle(&conversation);
            let services = shard.services();
            let package = services
                .user_streams
                .get(&name)
                .map(|binding| binding.package.clone());
            Ok(DirectAdmission {
                lease,
                package,
                resolver: services.native.resolver(&services.public_url),
            })
        })
    }

    fn package_event(&self, event: PackageEvent) -> LocalBoxFuture<'static, ()> {
        let shard = self.shard();
        let device = self.device.clone();
        Box::pin(async move {
            // A shard that is gone has no plugin to tell.
            let Ok(shard) = shard else { return };
            match event {
                PackageEvent::Message {
                    package,
                    conversation,
                    message,
                } => match reaching(&shard, &conversation, &device).await {
                    Ok(conversation) => {
                        shard
                            .plugins()
                            .package_message(&package, &conversation, message)
                            .await;
                    }
                    Err(error) => {
                        tracing::warn!(device = %device, %package, "a package message was dropped: {error}");
                    }
                },
                PackageEvent::Lost { package } => {
                    shard.package_lost(&device, Lost::Package(&package)).await;
                }
            }
        })
    }

    fn forward_event(&self, event: ForwardEvent) {
        // A shard that is closing serves no forward.
        if let Ok(shard) = self.shard() {
            shard.forward_event(&self.device, event);
        }
    }
}

/// `conversation`, a conversation of the user that reaches `device`, as its
/// primary Host or an attached one; or why it is none.
async fn reaching(shard: &Shard, conversation: &str, device: &DeviceId) -> Result<ConversationId, String> {
    let conversation = ConversationId::try_from(conversation)
        .map_err(|_| format!("no conversation {conversation}"))?;
    let hosts = reachable(shard.host_shard(), &conversation)
        .await
        .map_err(|error| error.to_string())?;
    if !hosts.iter().any(|host| &host.device == device) {
        return Err("the conversation does not reach this device".into());
    }
    Ok(conversation)
}
