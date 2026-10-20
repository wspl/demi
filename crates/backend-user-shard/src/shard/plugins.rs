//! The product's side of the plugins' port
//! (`crates/plugin-interface/src/lib.rs`).
//!
//! It serves:
//!
//! - package calls and reads of a conversation through its host access;
//! - the user's blobs;
//! - the Providers service's images.
//!
//! The plugin host answers the rest itself.

use std::rc::Weak;
use std::sync::Arc;

use bytes::Bytes;
use demi_backend_host_access::HostShard;
use demi_backend_host_access::access::HostAccessError;
use demi_backend_host_access::plugin_files::ReadFilesError;
use demi_backend_host_access::stream::{ServiceBinding, ServiceCall, UserCallError, UserCallKind};
use demi_backend_page_sync::Part;
use demi_backend_host_access::host_commands::reachable;
use demi_backend_plugins::{Lost, ProductPort, ReadOn, VideoJob};
use demi_backend_providers::vault::seal::Row;
use demi_backend_remote_host::ServiceCallError;
use demi_command_declarations::NativeOperation;
use demi_host_interface::{HostErrorKind, JobCaller, PortError};
use demi_plugin_interface::{
    AskedVideo, AudioAsk, AudioVoices, CallKind, HostFile, HostRead, ImageAsk, MadeAudio, MediaListing, MadeImage,
    PortFailure, PortRefusal, VideoAsk,
};
use demi_shared_types::{B64Bytes, BlobRef, MediaKind, Timestamp};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use super::Shard;
use crate::services::Services;

/// The most bytes of a package call's JSON answer.
const CALL_ANSWER_MAX_BYTES: usize = 1024 * 1024;

/// The shard as its plugins' port reaches it. Weak: the shard owns the
/// plugins that hold it.
pub(crate) struct ShardPort {
    pub(crate) shard: Weak<Shard>,
    pub(crate) services: Arc<Services>,
    /// The shard thread's client of the plugins' outbound requests.
    pub(crate) web: reqwest::Client,
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
        on: &'a ReadOn,
        reads: Vec<HostRead>,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<Vec<HostFile>, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            let hosts = shard.host_shard();
            let read = match on {
                ReadOn::Primary => hosts.read_files_on(conversation, None, &reads, cancel).await.map(|(files, _)| files),
                ReadOn::PrimaryOf(device) => hosts
                    .read_files_on(conversation, Some(device), &reads, cancel)
                    .await
                    .map(|(files, _)| files),
                ReadOn::Job { device, directory } => {
                    hosts.read_job_files(conversation, device, directory, &reads, cancel).await
                }
            };
            read.map_err(|error| match error {
                // A project search's Host that is no longer the
                // conversation's is not running for it.
                ReadFilesError::NotRunning | ReadFilesError::Moved => PortFailure::Refused(PortRefusal::NotRunning),
                ReadFilesError::Access(error) => access_refusal(error),
            })
        })
    }

    fn media_models(&self, kind: MediaKind) -> LocalBoxFuture<'_, Result<MediaListing, PortFailure>> {
        Box::pin(async move { super::media::media_models(&*self.shard()?, kind).await })
    }

    fn make_image<'a>(
        &'a self,
        conversation: &'a ConversationId,
        caller: Option<&'a JobCaller>,
        ask: ImageAsk,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<MadeImage, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            super::media::make_image(&shard, conversation, caller, ask, cancel).await
        })
    }

    fn make_video<'a>(
        &'a self,
        conversation: &'a ConversationId,
        caller: Option<&'a JobCaller>,
        ask: VideoAsk,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<(AskedVideo, VideoJob), PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            super::media::make_video(&shard, conversation, caller, ask, cancel).await
        })
    }

    fn make_audio<'a>(
        &'a self,
        conversation: &'a ConversationId,
        caller: Option<&'a JobCaller>,
        ask: AudioAsk,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<MadeAudio, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            super::media::make_audio(&shard, conversation, caller, ask, cancel).await
        })
    }

    fn audio_voices<'a>(
        &'a self,
        conversation: &'a ConversationId,
        model: String,
        search: Option<String>,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<AudioVoices, PortFailure>> {
        Box::pin(async move {
            let shard = self.shard()?;
            super::media::audio_voices(&shard, conversation, model, search, cancel).await
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

    fn seal(&self, user: &UserId, plugin: &str, name: &str, plaintext: &[u8]) -> Vec<u8> {
        let row = Row::PluginSecret { plugin, user, name };
        self.services.vault.key().seal(row, plaintext)
    }

    fn open(&self, user: &UserId, plugin: &str, name: &str, sealed: &[u8]) -> Option<Vec<u8>> {
        let row = Row::PluginSecret { plugin, user, name };
        self.services.vault.key().open(row, sealed).ok()
    }

    fn web_client(&self) -> reqwest::Client {
        self.web.clone()
    }

    fn public_url(&self) -> Option<url::Url> {
        self.services
            .public_url
            .get()
            .map(|backend| backend.url().clone())
    }

    fn now(&self) -> Timestamp {
        self.services.clock.now()
    }

    fn spawn(&self, work: LocalBoxFuture<'static, ()>) {
        // A shard already gone is shutting down: the work has not started,
        // and whoever waits for it hears that the backend is shutting down.
        if let Some(shard) = self.shard.upgrade() {
            shard.tasks().spawn_local(work);
        }
    }
}

/// The client of a shard thread's plugins' outbound requests
/// (`crates/backend-plugins/src/web.rs`). A test's backend trusts the
/// authorities its HTTPS servers present.
pub(crate) fn web_client(services: &Services) -> reqwest::Client {
    #[cfg(feature = "testing")]
    let roots = services.web_roots.as_slice();
    #[cfg(not(feature = "testing"))]
    let roots = {
        let _ = services;
        &[]
    };
    demi_backend_plugins::web_client(roots)
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

/// A package call that did not answer. The cases are:
///
/// - the host access's refusal when the Host cannot be reached or takes no
///   operations, as its routes answer it;
/// - a service that did not start, which the plugin names in its own words;
/// - a call whose connection to the Host broke partway, 409
///   `host_operation_failed`;
/// - otherwise the call's own failure, such as an answer over its bound, which a
///   page call answers as the plugin's failure.
fn call_failure(error: UserCallError) -> PortFailure {
    match error {
        UserCallError::Access(error) => access_refusal(error),
        UserCallError::Call(ServiceCallError::Exited { stderr, .. }) => {
            PortFailure::Refused(PortRefusal::Operation { stderr })
        }
        UserCallError::Call(ServiceCallError::NotOpened(message)) => {
            PortFailure::Refused(PortRefusal::Service { message })
        }
        UserCallError::Call(ServiceCallError::Host(error))
            if matches!(error.kind, HostErrorKind::Offline | HostErrorKind::Unavailable) =>
        {
            access_refusal(HostAccessError::Host(error))
        }
        UserCallError::Call(ServiceCallError::Host(error))
            if matches!(error.kind, HostErrorKind::Interrupted | HostErrorKind::Protocol) =>
        {
            PortFailure::Refused(PortRefusal::Host {
                code: ErrorCode::HostOperationFailed,
                status: 409,
                message: error.to_string(),
            })
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
    /// Turns `plugin` on or off for the user (`crates/backend-plugins/src/lib.rs`); every live
    /// tree's summary is marked, since whether a reload would change it may have changed.
    pub async fn switch_plugin(
        &self,
        plugin: &str,
        enabled: bool,
    ) -> Result<(), demi_backend_plugins::SwitchError> {
        if !self.plugins().switch(plugin, enabled).await? {
            return Ok(());
        }
        // Off, it keeps nothing; on, its next context request asks again.
        self.project_files().plugins.forget(|(kept, _)| kept.as_str() == plugin);
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

impl Shard {
    /// What `lost` names, kept on `device`, ended without its package
    /// saying so: each plugin of such a package with panel tabs in a
    /// conversation that reaches the device hears it, for that conversation
    /// (`crates/command-protocol/src/messages.rs`).
    pub(crate) async fn package_lost(&self, device: &DeviceId, lost: Lost<'_>) {
        self.cli_package_lost(device, &lost);
        let panelled = match self
            .services()
            .control
            .panelled_conversations(self.user().clone())
            .await
        {
            Ok(panelled) => panelled,
            Err(error) => {
                // The plugins then show what was lost until their packages
                // next say something about the conversations.
                tracing::warn!(device = %device, error = &error as &dyn std::error::Error, "the conversations a lost package showed in could not be read");
                return;
            }
        };
        let mut reaching = Vec::new();
        for conversation in panelled {
            match reachable(self.host_shard(), &conversation).await {
                Ok(hosts) if hosts.iter().any(|host| &host.device == device) => {
                    reaching.push(conversation);
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%conversation, "the Hosts of a conversation a lost package showed in could not be read: {error}");
                }
            }
        }
        self.plugins().package_lost(lost, &reaching).await;
    }
}
