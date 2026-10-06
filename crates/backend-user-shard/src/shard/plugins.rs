//! The product's side of the plugins' port (`plugins.md` § The contract):
//! package calls and reads of a conversation through its host access, and
//! the user's blobs. The plugin host answers the rest itself.

use std::rc::Weak;

use bytes::Bytes;
use demi_backend_host_access::HostShard;
use demi_backend_host_access::access::HostAccessError;
use demi_backend_host_access::plugin_files::ReadFilesError;
use demi_backend_host_access::stream::{ServiceBinding, ServiceCall, UserCallError, UserCallKind};
use demi_backend_page_sync::Part;
use demi_backend_plugins::ProductPort;
use demi_backend_remote_host::ServiceCallError;
use demi_command_declarations::NativeOperation;
use demi_host_interface::PortError;
use demi_plugin_interface::{CallKind, HostFile, HostRead, PortFailure, PortRefusal};
use demi_shared_types::{B64Bytes, BlobRef};
use demi_web_api_protocol::ids::ConversationId;
use futures_util::future::LocalBoxFuture;
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

    fn read_host_files<'a>(
        &'a self,
        conversation: &'a ConversationId,
        reads: Vec<HostRead>,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<Vec<HostFile>, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            shard
                .host_shard()
                .read_files(conversation, &reads, cancel)
                .await
                .map_err(|error| match error {
                    ReadFilesError::NotRunning => PortFailure::Refused(PortRefusal::NotRunning),
                    ReadFilesError::Access(error) => access_refusal(error),
                })
        })
    }

    fn put_blob(&self, bytes: B64Bytes) -> LocalBoxFuture<'_, Result<BlobRef, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            HostShard::blobs(&*shard)
                .put(bytes.into_bytes())
                .await
                .map_err(failed)
        })
    }

    fn get_blob(&self, blob: BlobRef) -> LocalBoxFuture<'_, Result<Option<B64Bytes>, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            let bytes = HostShard::blobs(&*shard).get(&blob).await.map_err(failed)?;
            Ok(bytes.map(B64Bytes::from))
        })
    }
}

fn failed(error: impl ToString) -> PortFailure {
    PortFailure::Port(PortError::Failed(error.to_string()))
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
