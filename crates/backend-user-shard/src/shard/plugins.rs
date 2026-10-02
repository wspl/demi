//! The product's side of the plugins' port (`plugins.md` § The contract):
//! package calls and the Hosts of a conversation through its host access,
//! and the user's exposes, which the shard holds. The plugin host answers
//! the rest itself.

use std::collections::HashMap;
use std::rc::Weak;

use bytes::Bytes;
use demi_backend_expose::records::{Expose, ExposeError};
use demi_backend_host_access::access::{self, HostAccessError};
use demi_backend_host_access::stream::{ServiceBinding, ServiceCall, UserCallError, UserCallKind};
use demi_backend_page_sync::Part;
use demi_backend_plugins::ProductPort;
use demi_backend_remote_host::ServiceCallError;
use demi_command_declarations::NativeOperation;
use demi_host_interface::PortError;
use demi_plugin_interface::{
    CallKind, ConversationHost, ExposeList, ExposeRecord, ExposeRefusal, Follows, HostRole,
    PortFailure, PortRefusal,
};
use demi_web_api_protocol::exposes::ExposeAddress;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, ExposeId};
use futures_util::future::LocalBoxFuture;
use jiff::SignedDuration;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use super::Shard;

/// The most bytes of a package call's JSON answer.
const CALL_ANSWER_MAX_BYTES: usize = 1024 * 1024;

/// The shard as its plugins' port reaches it. Weak: the shard owns the
/// plugins that hold it.
pub(crate) struct ShardPort {
    pub(crate) shard: Weak<Shard>,
}

impl ShardPort {
    fn shard(&self) -> Result<std::rc::Rc<Shard>, PortFailure> {
        self.shard
            .upgrade()
            .ok_or_else(|| PortError::Ended("the backend is shutting down".into()).into())
    }
}

impl ProductPort for ShardPort {
    fn package_call<'a>(
        &'a self,
        conversation: &'a ConversationId,
        operation: &'a NativeOperation,
        args: Map<String, Value>,
        kind: CallKind,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<Value, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            let package = shard
                .services()
                .native
                .package(&operation.package)
                .ok_or_else(|| failed("the catalog serves no such package"))?
                .clone();
            let call = ServiceCall {
                binding: ServiceBinding {
                    package,
                    operation: operation.operation.clone(),
                },
                args,
                max_bytes: CALL_ANSWER_MAX_BYTES,
            };
            let kind = match kind {
                CallKind::Starts => UserCallKind::Starts,
                CallKind::Operates => UserCallKind::Operates,
                CallKind::Looks => UserCallKind::Looks,
            };
            let answer: Bytes = shard
                .host_shard()
                .user_call(conversation, kind, &call, cancel)
                .await
                .map_err(call_failure)?;
            serde_json::from_slice(&answer).map_err(|error| {
                failed(format!("the operation answered what is not JSON: {error}"))
            })
        })
    }

    fn conversation_hosts<'a>(
        &'a self,
        conversation: &'a ConversationId,
    ) -> LocalBoxFuture<'a, Result<Vec<ConversationHost>, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            let hosts = shard
                .host_shard()
                .conversation_hosts(conversation)
                .await
                .map_err(access_refusal)?;
            Ok(hosts
                .into_iter()
                .map(|host| ConversationHost {
                    online: shard.devices().online(&host.device),
                    name: host.name,
                    device: host.device,
                    role: match host.role {
                        access::HostRole::Main => HostRole::Main,
                        access::HostRole::Attached => HostRole::Attached,
                    },
                })
                .collect())
        })
    }

    fn exposes(&self) -> LocalBoxFuture<'_, Result<ExposeList, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            let available = shard.services().expose_domain.is_some();
            let exposes = shard.expose_shard().list_exposes().await.map_err(failed)?;
            let mut names = HashMap::new();
            let mut records = Vec::new();
            for expose in exposes {
                let name = device_name(&shard, &mut names, &expose.record.device).await?;
                records.push(port_record(expose, name));
            }
            Ok(ExposeList {
                available,
                listed_at: shard.services().clock.now(),
                exposes: records,
            })
        })
    }

    fn create_expose(
        &self,
        device: DeviceId,
        address: String,
        lifetime: u64,
    ) -> LocalBoxFuture<'_, Result<ExposeRecord, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            let address = ExposeAddress::try_from(address).map_err(|error| {
                refused(
                    ExposeRefusal::InvalidAddress,
                    format!("the address {error}"),
                )
            })?;
            let expose = shard
                .expose_shard()
                .add_expose(&device, address, seconds(lifetime))
                .await
                .map_err(expose_failure)?;
            let name = device_name(&shard, &mut HashMap::new(), &device).await?;
            Ok(port_record(expose, name))
        })
    }

    fn renew_expose(
        &self,
        expose: ExposeId,
        lifetime: u64,
    ) -> LocalBoxFuture<'_, Result<ExposeRecord, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            let renewed = shard
                .expose_shard()
                .renew_expose(&expose, seconds(lifetime))
                .await
                .map_err(expose_failure)?;
            let device = renewed.record.device.clone();
            let name = device_name(&shard, &mut HashMap::new(), &device).await?;
            Ok(port_record(renewed, name))
        })
    }

    fn remove_expose(&self, expose: ExposeId) -> LocalBoxFuture<'_, Result<(), PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            shard
                .expose_shard()
                .remove_expose(&expose)
                .await
                .map_err(expose_failure)
        })
    }
}

impl Shard {
    /// Marks the page state of every plugin that follows the user's exposes
    /// as changed.
    pub(crate) fn mark_expose_followers(&self) {
        let marks = self.services().sync.of(self.user());
        for plugin in self.services().plugins.followers(Follows::Exposes) {
            marks.mark(Part::Plugin(plugin.as_str().to_owned()));
        }
    }
}

/// The name of `device`, read once per listing.
async fn device_name(
    shard: &Shard,
    names: &mut HashMap<DeviceId, String>,
    device: &DeviceId,
) -> Result<String, PortFailure> {
    if let Some(name) = names.get(device) {
        return Ok(name.clone());
    }
    let record = shard
        .services()
        .control
        .device(device.clone())
        .await
        .map_err(failed)?;
    let name = record.map_or_else(|| device.to_string(), |record| record.name);
    names.insert(device.clone(), name.clone());
    Ok(name)
}

fn port_record(expose: Expose, device_name: String) -> ExposeRecord {
    let Expose { record, url } = expose;
    ExposeRecord {
        id: record.id,
        device: record.device,
        device_name,
        address: record.address,
        url,
        created_at: record.created_at,
        expires_at: record.expires_at,
    }
}

fn seconds(lifetime: u64) -> SignedDuration {
    SignedDuration::from_secs(i64::try_from(lifetime).unwrap_or(i64::MAX))
}

fn failed(error: impl ToString) -> PortFailure {
    PortFailure::Port(PortError::Failed(error.to_string()))
}

fn refused(reason: ExposeRefusal, message: impl Into<String>) -> PortFailure {
    PortFailure::Refused(PortRefusal::Expose {
        reason,
        message: message.into(),
    })
}

fn expose_failure(error: ExposeError) -> PortFailure {
    let reason = match &error {
        ExposeError::Unavailable => ExposeRefusal::Unavailable,
        ExposeError::DeviceNotFound => ExposeRefusal::DeviceNotFound,
        ExposeError::DeviceOffline(_) => ExposeRefusal::DeviceOffline,
        ExposeError::NotFound(_) => ExposeRefusal::NotFound,
        ExposeError::Storage(_) => return failed(error),
    };
    refused(reason, error.to_string())
}

/// The conversation's host access refused, as its routes would answer.
fn access_refusal(error: HostAccessError) -> PortFailure {
    let (code, status) = error.code();
    PortFailure::Refused(PortRefusal::Host {
        code,
        status,
        message: error.to_string(),
    })
}

fn call_failure(error: UserCallError) -> PortFailure {
    match error {
        UserCallError::Access(error) => access_refusal(error),
        UserCallError::Call(ServiceCallError::Exited { stderr, .. }) => {
            PortFailure::Refused(PortRefusal::Operation { stderr })
        }
        UserCallError::Call(ServiceCallError::Host(error)) => {
            access_refusal(HostAccessError::Host(error))
        }
        UserCallError::Call(error) => failed(error),
    }
}

/// Why a conversation's tree was not reloaded.
#[derive(Debug, thiserror::Error)]
pub enum ReloadRefusal {
    #[error(transparent)]
    Access(#[from] HostAccessError),
    #[error("The conversation is archived")]
    Archived,
    #[error("The conversation's agents are working; reload once they are done")]
    Working,
}

impl Shard {
    /// Turns `plugin` on or off for the user (`plugins.md` § A user's
    /// plugins); every live tree's summary is marked, since whether a
    /// reload would change it may have changed.
    pub async fn switch_plugin(
        &self,
        plugin: &str,
        enabled: bool,
    ) -> Result<(), demi_backend_plugins::SwitchError> {
        if !self.plugins().switch(plugin, enabled).await? {
            return Ok(());
        }
        let marks = self.services().sync.of(self.user());
        for root in self.agent().live_roots() {
            marks.mark(Part::Conversation(
                demi_backend_host_access::conversation_of(&root),
            ));
        }
        Ok(())
    }

    /// Closes the conversation's live tree so that it opens again with the
    /// user's current plugins; a tree that works or an archived
    /// conversation is refused, and one with no live tree has nothing to do.
    pub async fn reload_conversation(&self, id: &ConversationId) -> Result<(), ReloadRefusal> {
        let record = self.host_shard().owned_conversation(id).await?;
        if record.archived {
            return Err(ReloadRefusal::Archived);
        }
        self.agent()
            .reload(&demi_backend_host_access::root_of(&record.id))
            .await
            .map_err(|_| ReloadRefusal::Working)
    }
}
