//! The backend's rules for the calls a runner connection relays.
//!
//! # Who may call
//!
//! A call runs only for a live job on the connection whose command context names
//! the conversation of the Host that started it, and it reaches that job's agent
//! node's commands.
//!
//! - A resident's conversation numbers come only from a conversation of
//!   the user that reaches the device
//!   (`crates/command-protocol/src/numbers.rs`).
//! - A direct stream counts as the activity of such a conversation only.
//! - A resident's message reaches its plugin only about such a conversation
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
//! connection, in the owner's core, rather than delegated to the generic Host
//! adapter, which knows no user policy.

use std::rc::{Rc, Weak};

use bytes::Bytes;
use demi_core_store::sequences;
use demi_core_host_access::host_commands::{reachable, resolve_host};
use demi_core_host_access::root_of;
use demi_plugin_host::Lost;
use demi_runner_client::{CommandCatalog, DirectAdmission, ForwardEvent, HostLog, JobOrigin, LinkPolicy, PackageEvent};
use demi_runner_protocol::wire::LinkedWorktree;
use demi_core_hosts::host_key::conversation_of;
use demi_core_permissions::Need;
use demi_command_protocol::ResidentSequence;
use demi_host_interface::{BroughtHost, RpcError, RpcInvocation, RpcPort};
use demi_shared_gates::{GateLease, Purpose};
use demi_shared_types::{BlobRef, Sequence};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;

use super::Core;

/// The rules of one device's connection, in its user's core.
pub(crate) struct CorePolicy {
    /// Weak: the core owns the connection this policy serves.
    core: Weak<Core>,
    device: DeviceId,
}

impl CorePolicy {
    pub fn new(core: &Rc<Core>, device: DeviceId) -> Self {
        Self {
            core: Rc::downgrade(core),
            device,
        }
    }

    fn core(&self) -> Result<Rc<Core>, String> {
        self.core
            .upgrade()
            .ok_or_else(|| "backend shutting down".to_owned())
    }
}

/// A demand lease of the conversation's stream gate, which holds it active
/// as a watched stream does, with its idle watch started, as a Host
/// admission starts it (`crates/core/src/lifecycle/mod.rs`).
async fn watches(core: &Core, conversation: &ConversationId) -> GateLease {
    let lease = core
        .conversations()
        .slot(conversation)
        .streams
        .enter(Purpose::Demand)
        .await;
    core.track_idle(conversation);
    lease
}

impl LinkPolicy for CorePolicy {
    fn programs(&self) -> Option<CommandCatalog> {
        let core = self.core.upgrade()?;
        let services = core.services();
        Some(services.packages.catalog())
    }

    fn host_log(&self) -> Option<Rc<dyn HostLog>> {
        self.core.upgrade()?.starter().host_log(&self.device)
    }

    fn admit_call<'a>(&'a self, job: &'a JobOrigin) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let conversation = match conversation_of(&job.host) {
                Some(conversation) if conversation == job.context.conversation => conversation,
                Some(_) => return Err("rpc job belongs to another conversation".into()),
                None => return Err("rpc requires a live job dispatched to this device".into()),
            };
            // Work a command left running on a Host that is no longer the
            // conversation's, such as one started there with `&` or `nohup`,
            // goes on as after an ssh session ends, but its calls are
            // refused. A conversation that cannot be read is refused by what
            // the call then needs.
            let core = self.core()?;
            let Ok(conversation) = ConversationId::try_from(conversation) else {
                return Ok(());
            };
            if let Ok(hosts) = core.host_core().conversation_hosts(&conversation).await
                && !hosts.iter().any(|host| host.device == self.device)
            {
                return Err("this host is no longer one of the conversation's".into());
            }
            Ok(())
        })
    }

    /// Runs the call in its node's commands.
    ///
    /// Once the command set checked the call, what it needs is checked against the
    /// conversation's grants before the handler runs: its leaf's category, or
    /// the device its `bringsHost` argument brings into the conversation, which
    /// the grant of the user's devices covers. A call that lacks it waits for the
    /// user's answer.
    fn dispatch(
        &self,
        job: Rc<JobOrigin>,
        invocation: RpcInvocation,
        port: RpcPort,
    ) -> LocalBoxFuture<'static, Result<u8, RpcError>> {
        let core = self.core();
        Box::pin(async move {
            let core = core.map_err(RpcError::Failed)?;
            let commands = core.commands().commands_of(&job).map_err(RpcError::Failed)?;
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
                        let host = resolve_host(core.host_core(), &conversation, named).await?;
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
                let need = match (&brought, checked.category) {
                    (Some(brought), _) => Some(Need::Device(
                        DeviceId::try_from(brought.device.clone())
                            .map_err(|_| RpcError::Failed("the resolved host has no id".into()))?,
                    )),
                    (None, Some(category)) => Some(Need::Category(category)),
                    (None, None) => None,
                };
                if let Some(need) = need {
                    demi_core_permissions::check(core.permission_core(), &invocation, need, &port).await?;
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
        let core = self.core();
        Box::pin(async move {
            core?
                .host_core()
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
        let core = self.core();
        let device = self.device.clone();
        Box::pin(async move {
            let core = core?;
            let conversation = core
                .store().running_jobs(device)
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .find(|recorded| recorded.sent && recorded.job == job)
                .map(|recorded| recorded.conversation)
                .ok_or("rpc requires a live job dispatched to this device")?;
            // Counted before the open, so the close waits for it before the
            // agent shuts down.
            let _opening = core.tree_openers().token();
            if core.is_closing() {
                return Err("the backend is shutting down".into());
            }
            core.restore_tree(&conversation).await?;
            core.agent().take_up_job(&root_of(&conversation), &job).await
        })
    }

    /// Revokes the device at its runner's request, as the user's
    /// revocation does: only a paired device.
    fn revoke_device(&self) -> LocalBoxFuture<'static, Result<(), String>> {
        let core = self.core();
        let device = self.device.clone();
        Box::pin(async move {
            let core = core?;
            let record = core
                .store().device(device.clone())
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "the device is gone".to_owned())?;
            if record.kind != DeviceKind::User {
                return Err("only a paired device is revoked".into());
            }
            core
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
        sequence: ResidentSequence,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<u64, String>> {
        let core = self.core();
        let device = self.device.clone();
        Box::pin(async move {
            let core = core?;
            let conversation = reaching(&core, &conversation, &device).await?;
            let sequence = match sequence {
                ResidentSequence::Tab => Sequence::Tab,
                ResidentSequence::Device => Sequence::Device,
            };
            core.store().databases()
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
    /// (`crates/core/src/lifecycle/mod.rs`). It installs what a
    /// relay stream of the user stream `name` may: the executable of the package
    /// the name binds, which the runner fetches from where the published catalog
    /// says.
    fn direct_stream(
        &self,
        conversation: String,
        name: String,
    ) -> LocalBoxFuture<'static, Result<DirectAdmission, String>> {
        let core = self.core();
        let device = self.device.clone();
        Box::pin(async move {
            let core = core?;
            let conversation = reaching(&core, &conversation, &device).await?;
            let lease = watches(&core, &conversation).await;
            let services = core.services();
            let package = services
                .user_streams
                .get(&name)
                .map(|binding| binding.package.clone());
            Ok(DirectAdmission {
                lease,
                package,
                resolver: services.packages.resolver(),
            })
        })
    }

    /// Holds the conversation active again, as [`Self::direct_stream`]'s
    /// admission does.
    fn watch_stream(&self, conversation: String) -> LocalBoxFuture<'static, Result<GateLease, String>> {
        let core = self.core();
        let device = self.device.clone();
        Box::pin(async move {
            let core = core?;
            let conversation = reaching(&core, &conversation, &device).await?;
            Ok(watches(&core, &conversation).await)
        })
    }

    fn package_event(&self, event: PackageEvent) -> LocalBoxFuture<'static, ()> {
        let core = self.core();
        let device = self.device.clone();
        Box::pin(async move {
            // A core that is gone has no plugin to tell.
            let Ok(core) = core else { return };
            match event {
                PackageEvent::Message {
                    package,
                    conversation,
                    message,
                } => match reaching(&core, &conversation, &device).await {
                    Ok(conversation) => {
                        core
                            .plugins()
                            .package_message(&package, &conversation, &device, message)
                            .await;
                    }
                    Err(error) => {
                        tracing::warn!(event = "package_event.dropped", host_id = %device, %package, cause = "conversation_unreachable", %error, "a package message was dropped");
                    }
                },
                PackageEvent::Lost { package } => {
                    core.package_lost(&device, Lost::Package(&package)).await;
                }
            }
        })
    }

    fn forward_event(&self, event: ForwardEvent) {
        // A core that is closing serves no forward.
        if let Ok(core) = self.core() {
            core.forward_event(&self.device, event);
        }
    }

    fn worktrees(&self, directory: String, worktrees: Vec<LinkedWorktree>) {
        // A core that is closing stores nothing; the next connection
        // reports again.
        if let Ok(core) = self.core() {
            core.worktrees_reported(&self.device, directory, worktrees);
        }
    }
}

/// `conversation`, a conversation of the user that reaches `device`, as its
/// primary Host or an attached one; or why it is none.
async fn reaching(core: &Core, conversation: &str, device: &DeviceId) -> Result<ConversationId, String> {
    let conversation = ConversationId::try_from(conversation)
        .map_err(|_| format!("no conversation {conversation}"))?;
    let hosts = reachable(core.host_core(), &conversation)
        .await
        .map_err(|error| error.to_string())?;
    if !hosts.iter().any(|host| &host.device == device) {
        return Err("the conversation does not reach this device".into());
    }
    Ok(conversation)
}
