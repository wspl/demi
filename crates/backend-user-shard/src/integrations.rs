//! A user's integrations (`crates/backend-database/src/integrations.rs`): the
//! programs that build an agent product on Demi through its API, which the
//! user makes and changes in Settings › Integrations.
//!
//! An integration infers with every provider its user has, and its
//! conversations are in a workspace of its own that no page lists.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use demi_agent_server::{CommandMissed, Observer, TreeObservers};
use demi_agent_tools::parse_workflow;
use demi_backend_host_access::{conversation_of, root_of};
use demi_conversation_lane_protocol::ServerFrame;
use demi_host_interface::PortResponse;
use demi_shared_types::{B64Bytes, CommandId, NodeId};
use demi_web_api_protocol::cloud::CloudState;
use demi_web_api_protocol::devices::DeviceKind;
use futures_util::future::LocalBoxFuture;
use tokio::sync::{Notify, mpsc};

use demi_backend_database::StorageError;
use demi_backend_page_sync::Part;
use demi_backend_database::usage::Attribution;
use demi_machine_manager_protocol::shared::mount_path;
use demi_backend_database::conversation_index::{
    ConversationChange, ConversationRecord, ConversationStart, Creation, RecordChange, SettingsChange,
};
use demi_backend_database::integrations::{
    InitState, IntegrationCloud, IntegrationConversation, IntegrationPatch as StoredPatch, IntegrationRecord,
};
use demi_backend_host_access::transition::ChangeRefusal;
use demi_conversation_lane_protocol::{AgentEntry, AgentRole, ClientContent, ClientFrame, Refusal, SendMessage};
use demi_integration_api_protocol as api;
use demi_shared_types::{BlockId, Timestamp, TurnId, UserContentBlock, WorkflowChoice};
use demi_web_api_protocol::conversations::{ConversationStatus, ConversationTarget, ModelChoice, ModelSettings};
use demi_web_api_protocol::error::ErrorCode;

use demi_web_api_protocol::ids::{ConversationId, DeviceId, IntegrationId, OperationId, ProviderId};
use demi_backend_cloud::machine::SHARED_FOLDER_GB;
use demi_web_api_protocol::users::CloudStorage;
use demi_web_api_protocol::integrations::{IntegrationDto, IntegrationPatch, IntegrationWithToken, ServerSizes};
use tokio_util::sync::CancellationToken;

use crate::conversation::ForkRefusal;
use crate::conversation::outcome_of;
use crate::shard::Shard;

impl Shard {
    /// The sizes an integration's Cloud gets from the server.
    pub fn server_sizes(&self) -> ServerSizes {
        let services = self.services();
        let resources = services.cloud.machines.resources();
        let host = resources.map(|resources| resources.host);
        // A manager that limits no Cloud gives each the whole host.
        let cloud = resources.map(|resources| resources.cloud.unwrap_or(resources.host));
        let mebibytes = |memory_mib: u64| u32::try_from(memory_mib).unwrap_or(u32::MAX);
        ServerSizes {
            cpus: cloud.map(|cloud| cloud.cpus),
            memory_mib: cloud.map(|cloud| mebibytes(cloud.memory_mib)),
            system_gb: CloudStorage::DEFAULT.system_gb,
            home_gb: CloudStorage::DEFAULT.home_gb,
            shared_gb: SHARED_FOLDER_GB,
            idle_minutes: u32::try_from(services.lifecycle.idle_window.as_secs() / 60).unwrap_or(u32::MAX),
            server_cpus: host.map(|host| host.cpus),
            server_memory_mib: host.map(|host| mebibytes(host.memory_mib)),
        }
    }

    /// The user's integrations, oldest first.
    pub async fn integrations(&self) -> Result<Vec<IntegrationDto>, StorageError> {
        let control = &self.services().control;
        let mut shown = Vec::new();
        for record in control.integrations(self.user().clone()).await? {
            shown.push(self.integration_dto(record).await?);
        }
        Ok(shown)
    }

    /// Add Integration: one at once, every setting at its default, with the
    /// token it is shown with this once.
    pub async fn create_integration(&self) -> Result<IntegrationWithToken, StorageError> {
        let (record, token) = self.services().control.create_integration(self.user().clone()).await?;
        self.mark_integrations();
        Ok(IntegrationWithToken {
            integration: self.integration_dto(record).await?,
            token,
        })
    }

    /// Applies `patch` to the user's integration `id`; none for one that is
    /// not the user's.
    pub async fn update_integration(
        &self,
        id: IntegrationId,
        patch: IntegrationPatch,
    ) -> Result<Option<IntegrationDto>, IntegrationRefusal> {
        if self.own_integration(&id).await?.is_none() {
            return Ok(None);
        }
        let stored = StoredPatch {
            name: patch.name.map(|name| name.into_string()),
            capabilities: patch.capabilities,
            init_script: patch.init_script,
            shared_path: patch.shared_path,
            cloud_sizing: patch.cloud_sizing,
            cloud_idle_minutes: patch.cloud_idle_minutes,
        };
        match self.change_integration(id, stored).await? {
            Some(record) => Ok(Some(self.integration_dto(record).await?)),
            None => Ok(None),
        }
    }

    /// Applies `patch` to the integration `id`, as its page or its program
    /// asks: a script of blanks is none, and a shared folder's path is
    /// one a Cloud can mount. Answers the integration as it is now; none
    /// for one that is gone.
    pub async fn change_integration(
        &self,
        id: IntegrationId,
        mut patch: StoredPatch,
    ) -> Result<Option<IntegrationRecord>, IntegrationRefusal> {
        if let Some(Some(script)) = &patch.init_script
            && script.trim().is_empty()
        {
            patch.init_script = Some(None);
        }
        if let Some(path) = &patch.shared_path {
            mount_path(path).map_err(IntegrationRefusal::Invalid)?;
        }
        let changed = self.services().control.update_integration(id, patch).await?;
        if let Some(integration) = &changed
            && !integration.capabilities.browser
        {
            self.services().views.withdraw(&integration.id);
        }
        self.mark_integrations();
        Ok(changed)
    }

    /// The integrations, their settings or their counts of Clouds and
    /// conversations, changed: the master's pages receive them.
    pub(crate) fn mark_integrations(&self) {
        self.services().sync.mark(self.user(), Part::Integrations);
    }

    /// Replace Token: a new token for the user's integration `id`, which the
    /// old one no longer opens; none for one that is not the user's.
    pub async fn replace_integration_token(&self, id: IntegrationId) -> Result<Option<IntegrationWithToken>, StorageError> {
        if self.own_integration(&id).await?.is_none() {
            return Ok(None);
        }
        let Some(token) = self.services().control.replace_integration_token(id.clone()).await? else {
            return Ok(None);
        };
        // Read again, with the new token's hash.
        let Some(record) = self.own_integration(&id).await? else {
            return Ok(None);
        };
        // The sockets the old token opened end with it.
        self.integration_events.close(&id);
        self.mark_integrations();
        Ok(Some(IntegrationWithToken {
            integration: self.integration_dto(record).await?,
            token,
        }))
    }

    /// The user's integration `id`; none for one of another user's or none.
    pub(crate) async fn own_integration(&self, id: &IntegrationId) -> Result<Option<IntegrationRecord>, StorageError> {
        Ok(self
            .services()
            .control
            .integration(id.clone())
            .await?
            .filter(|record| record.user == *self.user()))
    }

    async fn integration_dto(&self, record: IntegrationRecord) -> Result<IntegrationDto, StorageError> {
        let (clouds, conversations) = self.services().control.integration_counts(record.id.clone()).await?;
        Ok(IntegrationDto {
            id: record.id,
            name: record.name,
            token_fingerprint: record.token_hash.as_str().chars().take(8).collect(),
            capabilities: record.capabilities,
            created_at: record.created_at,
            init_script: record.init_script,
            shared_path: record.shared_path,
            cloud_sizing: record.cloud_sizing,
            cloud_idle_minutes: record.cloud_idle_minutes,
            clouds,
            conversations,
        })
    }
}

/// Why an integration's request changed nothing, in words for the program.
#[derive(Debug, thiserror::Error)]
pub enum IntegrationRefusal {
    #[error("the integration has no {0} of that id")]
    NotFound(&'static str),
    #[error("{0}")]
    Refused(String),
    /// A value the request gave that no integration takes.
    #[error("{0}")]
    Invalid(String),
    /// A steer names another Cloud than the one the running conversation
    /// works on.
    #[error("the conversation is running on another Cloud: send the message without `steer`")]
    CloudDiffers,
    /// The Cloud a request reaches did not start.
    #[error(transparent)]
    Cloud(#[from] demi_backend_cloud::machine::CloudError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// A conversation's store failed to read what it holds.
    #[error(transparent)]
    ConversationStore(demi_agent_server::CommandMissed),
    /// What Demi keeps could not be read, as the message says.
    #[error("{0}")]
    Internal(String),
}

impl From<ChangeRefusal> for IntegrationRefusal {
    fn from(refusal: ChangeRefusal) -> Self {
        Self::Refused(refusal.to_string())
    }
}

impl From<Refusal> for IntegrationRefusal {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal.reason)
    }
}

/// How many conversations a page of `GET /v1/conversations` holds when the
/// request names no limit: one answer reads each one's checkpoint, so it
/// stays bounded, and the program asks for the rest by `next`.
const CONVERSATION_PAGE: u32 = 100;

/// A model choice's refusal: one that names no model, or an effort or a
/// tier the model does not offer, is the request's mistake.
fn model_refused(refusal: ChangeRefusal) -> IntegrationRefusal {
    match refusal {
        ChangeRefusal::ProviderNotFound | ChangeRefusal::ModelNotFound | ChangeRefusal::SettingUnavailable(_) => {
            IntegrationRefusal::Invalid(refusal.to_string())
        }
        refusal => refusal.into(),
    }
}

/// Refuses a workflow that does not read as a workflow file, which its
/// agents would otherwise run without, told only in their instructions.
fn checked_workflow(workflow: Option<&str>) -> Result<(), IntegrationRefusal> {
    match workflow.map(parse_workflow) {
        Some(Err(error)) => Err(IntegrationRefusal::Invalid(format!("`workflow` is no workflow: {}", error.reason()))),
        _ => Ok(()),
    }
}

/// The creation and the id a page's `next` names, `<ms>.<id>`.
fn page_cursor(cursor: &str) -> Option<(Timestamp, ConversationId)> {
    let (at, id) = cursor.split_once('.')?;
    let at = Timestamp::from_millisecond(at.parse().ok()?).ok()?;
    Some((at, ConversationId::try_from(id).ok()?))
}

/// The name a conversation's own workflow goes by, which its integration
/// passed when it made the conversation.
pub(crate) const INTEGRATION_WORKFLOW: &str = "the conversation's workflow";

/// Where a conversation of an integration works on its Cloud: the Cloud's
/// home, the same on every Cloud it moves to.
const CONVERSATION_DIRECTORY: &str = "~";

impl Shard {
    /// The integration's Clouds, oldest first, each in its state now.
    pub async fn integration_clouds(&self, integration: &IntegrationRecord) -> Result<Vec<api::Cloud>, StorageError> {
        let control = &self.services().control;
        let mut clouds = Vec::new();
        for cloud in control.integration_clouds(integration.id.clone()).await? {
            if let Some(shown) = self.integration_cloud_shown(cloud).await? {
                clouds.push(shown);
            }
        }
        Ok(clouds)
    }

    /// The integration's Cloud `device`; none for one it does not have.
    pub async fn integration_cloud(
        &self,
        integration: &IntegrationRecord,
        device: &DeviceId,
    ) -> Result<Option<api::Cloud>, StorageError> {
        match self.services().control.integration_cloud(device.clone()).await? {
            Some(cloud) if cloud.integration == integration.id => self.integration_cloud_shown(cloud).await,
            _ => Ok(None),
        }
    }

    /// Makes a Cloud of the integration's, or with `key` answers the one that
    /// has it, as it is. It boots at its first use, or now when it is always
    /// on.
    pub async fn create_integration_cloud(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        request: api::CreateCloud,
    ) -> Result<api::Cloud, IntegrationRefusal> {
        let _lease = self
            .integration_deletions
            .lease(&[integration.id.as_str()])
            .ok_or_else(|| IntegrationRefusal::Refused("the integration or its Cloud is being deleted".to_owned()))?;
        let cloud = self
            .services()
            .control
            .create_integration_cloud(integration.id.clone(), request.key, request.always_on)
            .await?;
        self.mark_integrations();
        if cloud.always_on {
            self.start_cloud(&cloud.device);
        }
        self.integration_cloud_shown(cloud)
            .await?
            .ok_or(IntegrationRefusal::NotFound("Cloud"))
    }

    /// Changes the integration's Cloud `device`: one that turns always on
    /// starts now.
    pub async fn patch_integration_cloud(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        device: &DeviceId,
        patch: api::CloudPatch,
    ) -> Result<api::Cloud, IntegrationRefusal> {
        if self.integration_cloud(integration, device).await?.is_none() {
            return Err(IntegrationRefusal::NotFound("Cloud"));
        }
        if let Some(always_on) = patch.always_on {
            self.services().control.set_cloud_always_on(device.clone(), always_on).await?;
            if always_on {
                self.start_cloud(device);
            }
        }
        self.integration_cloud(integration, device)
            .await?
            .ok_or(IntegrationRefusal::NotFound("Cloud"))
    }

    /// Starts the Cloud `device` in the background, as its first use would,
    /// with its init script.
    pub fn start_cloud(self: &Rc<Self>, device: &DeviceId) {
        let shard = self.clone();
        let device = device.clone();
        self.tasks().spawn_local(async move {
            let Ok(Some(record)) = shard.services().control.device(device.clone()).await else {
                return;
            };
            let started = async {
                let _access = shard.cloud_shard().access(record.clone()).await?;
                shard.ready_machine(&record).await;
                Ok::<_, demi_backend_cloud::machine::CloudError>(())
            };
            tokio::select! {
                started = started => {
                    if let Err(error) = started {
                        tracing::warn!(event = "cloud.always_on_start_failed", host_id = %device, %error, "an always-on Cloud did not start");
                    }
                }
                () = shard.closed() => {}
            }
        });
    }

    /// Resets the integration's Cloud `device` now: its system is rebuilt,
    /// `/home` stays.
    pub async fn reset_integration_cloud(
        &self,
        integration: &IntegrationRecord,
        device: &DeviceId,
    ) -> Result<api::Cloud, IntegrationRefusal> {
        if self.integration_cloud(integration, device).await?.is_none() {
            return Err(IntegrationRefusal::NotFound("Cloud"));
        }
        let record = self
            .services()
            .control
            .device(device.clone())
            .await?
            .ok_or(IntegrationRefusal::NotFound("Cloud"))?;
        let operation = OperationId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.end_cloud_commands(device).await;
        self.cloud_shard()
            .reset_machine(&record, operation)
            .await
            .map_err(|error| IntegrationRefusal::Refused(error.to_string()))?;
        // Its new system runs the script again; a reset refused leaves the
        // Cloud owing what it owed. The reset holds the Cloud, so no work
        // reaches it before this is written.
        self.services().control.rerun_cloud_init(device.clone()).await?;
        self.integration_cloud(integration, device)
            .await?
            .ok_or(IntegrationRefusal::NotFound("Cloud"))
    }

    async fn integration_cloud_shown(&self, cloud: IntegrationCloud) -> Result<Option<api::Cloud>, StorageError> {
        let Some(device) = self.services().control.device(cloud.device.clone()).await? else {
            return Ok(None);
        };
        let state = self.cloud_shard().machine_state(&device).await?;
        let scripted = self
            .services()
            .control
            .integration(cloud.integration.clone())
            .await?
            .is_some_and(|integration| integration.init_script.is_some());
        let run = match cloud.init.state {
            InitState::Pending => Some(api::InitState::Pending),
            InitState::Running => Some(api::InitState::Running),
            InitState::Done => Some(api::InitState::Done),
            InitState::Failed => Some(api::InitState::Failed),
            InitState::Unscripted => None,
        };
        let init = match run.filter(|_| scripted) {
            Some(state) => Some(api::CloudInit {
                state,
                exit_code: cloud.init.exit_code,
                output: self.services().control.cloud_init_output(cloud.device.clone()).await?,
            }),
            None => None,
        };
        Ok(Some(api::Cloud {
            id: cloud.device,
            key: cloud.key,
            state,
            init,
            always_on: cloud.always_on,
        }))
    }

    /// The integration's conversations, newest first.
    pub async fn integration_conversations(
        &self,
        integration: &IntegrationRecord,
    ) -> Result<Vec<api::Conversation>, StorageError> {
        let control = &self.services().control;
        let mut shown = Vec::new();
        for kept in control.integration_conversations(integration.id.clone()).await? {
            if let Some(record) = control.conversation(kept.conversation.clone()).await? {
                shown.push(self.integration_conversation_shown(record, kept.key).await?);
            }
        }
        Ok(shown)
    }

    /// A page of the integration's conversations, newest first, as `query`
    /// asks: with a key, the one of that key or none.
    pub async fn integration_conversation_page(
        &self,
        integration: &IntegrationRecord,
        query: api::ConversationQuery,
    ) -> Result<api::ConversationList, IntegrationRefusal> {
        let control = &self.services().control;
        if let Some(key) = query.key {
            let conversations = match control.integration_conversation_by_key(integration.id.clone(), key).await? {
                Some(id) => vec![self.integration_conversation_of(integration, &id).await?],
                None => Vec::new(),
            };
            return Ok(api::ConversationList { conversations, next: None });
        }
        let after = match query.after.as_deref() {
            Some(cursor) => Some(page_cursor(cursor).ok_or_else(|| IntegrationRefusal::Invalid("`after` is no page's `next`.".to_owned()))?),
            None => None,
        };
        let limit = query.limit.unwrap_or(CONVERSATION_PAGE);
        let page = control.integration_conversation_page(integration.id.clone(), after, limit).await?;
        let next = (page.len() == limit as usize)
            .then(|| page.last().map(|(kept, at)| format!("{}.{}", at.as_millisecond(), kept.conversation)))
            .flatten();
        let mut conversations = Vec::new();
        for (kept, _) in page {
            if let Some(record) = control.conversation(kept.conversation.clone()).await? {
                conversations.push(self.integration_conversation_shown(record, kept.key).await?);
            }
        }
        Ok(api::ConversationList { conversations, next })
    }

    /// Makes a conversation of the integration's on its Cloud `request.cloud`,
    /// or with a key answers the one that has it, as it is.
    pub async fn create_integration_conversation(
        &self,
        integration: &IntegrationRecord,
        request: api::CreateConversation,
    ) -> Result<api::Conversation, IntegrationRefusal> {
        let control = &self.services().control;
        checked_workflow(request.workflow.as_deref())?;
        if let Some(key) = &request.key
            && let Some(id) = control.integration_conversation_by_key(integration.id.clone(), key.clone()).await?
        {
            return self.integration_conversation_of(integration, &id).await;
        }
        let _lease = self
            .integration_deletions
            .lease(&[integration.id.as_str(), request.cloud.as_str()])
            .ok_or_else(|| IntegrationRefusal::Refused("the integration or its Cloud is being deleted".to_owned()))?;
        if self.integration_cloud(integration, &request.cloud).await?.is_none() {
            return Err(IntegrationRefusal::NotFound("Cloud"));
        }
        let model = self
            .chosen_selection(&ModelSettings {
                provider_id: ProviderId::try_from(request.model.provider_id.clone())
                    .map_err(|_| IntegrationRefusal::Invalid("No provider has that id.".to_owned()))?,
                model_id: request.model.model_id.clone(),
                thinking_effort: request.model.effort.clone(),
                service_tier_id: request.model.service_tier.clone(),
            })
            .await
            .map_err(model_refused)?;
        let id = ConversationId::try_from(uuid::Uuid::new_v4().to_string().as_str()).expect("a UUID is a conversation id");
        let start = ConversationStart {
            workspace: integration.workspace.clone(),
            title: None,
            pinned: false,
            target: ConversationTarget::Device {
                device_id: request.cloud.clone(),
                path: request.directory.clone().unwrap_or_else(|| CONVERSATION_DIRECTORY.to_owned()),
            },
            model: Some(model),
            workflow: WorkflowChoice::None,
        };
        let record = match self.create_started(id.clone(), start).await? {
            Creation::Created(record) | Creation::Existing(record) => record,
            Creation::Unavailable => return Err(IntegrationRefusal::Refused("The conversation id is taken.".to_owned())),
            Creation::Unselectable(reason) => return Err(ChangeRefusal::from(reason).into()),
        };
        let kept = IntegrationConversation {
            conversation: record.id.clone(),
            integration: integration.id.clone(),
            key: request.key.clone(),
            workflow: request.workflow.clone(),
            identity: request.identity.clone(),
            commands: request.commands.clone(),
            browser_profile: request.browser_profile.clone(),
        };
        // Two requests with one key made two conversations: the first kept
        // is the key's, and this one goes.
        if let Some(first) = control.record_integration_conversation(kept).await? {
            let _ = self.delete_conversation(&record.id).await;
            return self.integration_conversation_of(integration, &first).await;
        }
        self.mark_integrations();
        self.integration_conversation_shown(record, request.key).await.map_err(Into::into)
    }

    /// The integration's conversation `id` with its agents.
    pub async fn integration_conversation_detail(
        &self,
        integration: &IntegrationRecord,
        id: &ConversationId,
    ) -> Result<api::ConversationDetail, IntegrationRefusal> {
        let conversation = self.integration_conversation_of(integration, id).await?;
        let (_, entries) = self
            .conversation_agents(&conversation.id)
            .await
            .map_err(|error| IntegrationRefusal::Refused(error.to_string()))?;
        let agents = entries.into_iter().filter_map(api_agent).collect();
        Ok(api::ConversationDetail { conversation, agents })
    }

    /// Moves the integration's conversation `id` to another of its Clouds,
    /// renames it, or changes its model, its workflow, its identity or its
    /// commands.
    ///
    /// What the patch names is checked before anything changes, so a
    /// refusal of its workflow, its Cloud or its model leaves the
    /// conversation as it was. The move goes last, since only it can be
    /// refused after its checks, by another move that came first.
    pub async fn patch_integration_conversation(
        &self,
        integration: &IntegrationRecord,
        id: &ConversationId,
        patch: api::ConversationPatch,
    ) -> Result<api::Conversation, IntegrationRefusal> {
        let id = &self.integration_conversation_of(integration, id).await?.id;
        if let Some(Some(workflow)) = &patch.workflow {
            checked_workflow(Some(workflow))?;
        }
        let moves = patch.cloud.is_some() || patch.directory.is_some();
        // Held until the move is written.
        let mut _lease = None;
        if let Some(cloud) = &patch.cloud {
            _lease = Some(
                self.integration_deletions
                    .lease(&[cloud.as_str()])
                    .ok_or_else(|| IntegrationRefusal::Refused("the integration or its Cloud is being deleted".to_owned()))?,
            );
            if self.integration_cloud(integration, cloud).await?.is_none() {
                return Err(IntegrationRefusal::NotFound("Cloud"));
            }
        }
        if let Some(model) = patch.model {
            let change = SettingsChange {
                model: Some(ModelChoice {
                    provider_id: ProviderId::try_from(model.provider_id)
                        .map_err(|_| IntegrationRefusal::Invalid("No provider has that id.".to_owned()))?,
                    model_id: model.model_id,
                }),
                thinking_effort: model.effort,
                service_tier_id: Some(model.service_tier),
            };
            self.transition(id, ConversationChange::Settings(change))
                .await
                .map_err(model_refused)?;
        }
        if let Some(workflow) = patch.workflow {
            self.services().control.set_integration_workflow(id.clone(), workflow).await?;
        }
        if let Some(identity) = patch.identity {
            self.services().control.set_integration_identity(id.clone(), identity).await?;
            self.agent().adopt_identity(&root_of(id)).await;
        }
        if let Some(profile) = patch.browser_profile {
            self.services().control.set_integration_browser_profile(id.clone(), profile).await?;
        }
        if let Some(commands) = patch.commands {
            self.services().control.set_integration_commands(id.clone(), commands).await?;
            self.agent()
                .adopt_commands_of(&root_of(id))
                .await
                .map_err(IntegrationRefusal::Refused)?;
        }
        if let Some(title) = patch.title {
            self.transition(id, RecordChange::Title(title).into()).await?;
        }
        if moves {
            let (device, directory) = self.integration_place(id).await?;
            let target = ConversationTarget::Device {
                device_id: patch.cloud.or(device).ok_or(IntegrationRefusal::NotFound("Cloud"))?,
                path: patch.directory.unwrap_or(directory),
            };
            self.move_to(id, target, &CancellationToken::new()).await?;
        }
        self.integration_conversation_of(integration, id).await
    }

    /// Whether the integration's conversation `id` runs, as its `status`
    /// says `running`.
    fn integration_conversation_running(&self, id: &ConversationId) -> bool {
        self.agent().tree(&root_of(id)).is_some_and(|tree| tree.is_running())
    }

    /// Where the integration's conversation `id` works: its Cloud, none for
    /// one that is gone, and its directory as the integration names it, `~`
    /// for its Cloud's home.
    async fn integration_place(&self, id: &ConversationId) -> Result<(Option<DeviceId>, String), StorageError> {
        let Some(record) = self.services().control.conversation(id.clone()).await? else {
            return Ok((None, CONVERSATION_DIRECTORY.to_owned()));
        };
        let Ok(target) = self.host_shard().resolve_target(&record).await else {
            return Ok((None, CONVERSATION_DIRECTORY.to_owned()));
        };
        let device = target.device().cloned();
        let record = match &device {
            Some(device) => self.services().control.device(device.clone()).await?,
            None => None,
        };
        // The home a `~` reaches there, as a move reads it: a Cloud before
        // its first start has the one every Cloud has.
        let home = record.as_ref().and_then(|record| match record.kind {
            DeviceKind::Managed => Some(record.home.clone().unwrap_or_else(|| demi_backend_database::CLOUD_HOME.to_owned())),
            DeviceKind::User => record.home.clone(),
        });
        let directory = match (&record, home) {
            (Some(record), Some(home)) => record.platform.paths().written_from_home(&home, target.path()),
            _ => target.path().to_owned(),
        };
        Ok((device.filter(|_| record.is_some()), directory))
    }

    /// Moves the integration's conversation `id` to its Cloud `device` before
    /// a turn whose message named it, keeping its directory; nothing when it
    /// works there already. `cancel` ends it before it commits.
    pub(crate) async fn move_for_turn(&self, id: &ConversationId, device: &str, cancel: &CancellationToken) -> Result<(), String> {
        let device = DeviceId::try_from(device.to_owned()).map_err(|_| "no Cloud has that id".to_owned())?;
        // Held until the move is written, as a PATCH's.
        let _lease = self
            .integration_deletions
            .lease(&[device.as_str()])
            .ok_or_else(|| "the Cloud is being deleted".to_owned())?;
        let (_, directory) = self.integration_place(id).await.map_err(|error| error.to_string())?;
        let target = ConversationTarget::Device {
            device_id: device,
            path: directory,
        };
        self.move_to(id, target, cancel)
            .await
            .map(drop)
            .map_err(|refusal| refusal.to_string())
    }

    /// Sends a message to the integration's conversation `id`: a turn of its
    /// own, queued while one runs, or with `steer` a steer of the running
    /// turn.
    pub async fn send_integration_message(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        id: &ConversationId,
        message: api::SendMessage,
    ) -> Result<api::MessageAnswer, IntegrationRefusal> {
        let conversation = self.integration_conversation_of(integration, id).await?;
        let id = &conversation.id;
        if let Some(cloud) = &message.cloud
            && self.integration_cloud(integration, cloud).await?.is_none()
        {
            return Err(IntegrationRefusal::NotFound("Cloud"));
        }
        // A steer of a running turn runs where that turn does; one into an
        // idle agent starts a turn, which moves as a message's does.
        let elsewhere = message.cloud.as_ref().is_some_and(|cloud| conversation.cloud.as_ref() != Some(cloud));
        let steer = message.steer && !(elsewhere && !self.integration_conversation_running(id));
        if steer && elsewhere {
            return Err(IntegrationRefusal::CloudDiffers);
        }
        let message_id = message.id.unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        let mut content = vec![ClientContent::Text { text: message.text }];
        content.extend(message.attachments.into_iter().map(|file| ClientContent::Upload {
            r#ref: file.id,
            file_name: file.name,
        }));
        let frame = if steer {
            ClientFrame::Steer {
                steer_id: BlockId::try_from(message_id.clone())
                    .map_err(|_| IntegrationRefusal::Invalid("The message id is no id.".to_owned()))?,
                content,
                comments: Vec::new(),
            }
        } else {
            ClientFrame::Send(SendMessage {
                message_id: TurnId::try_from(message_id.clone())
                    .map_err(|_| IntegrationRefusal::Invalid("The message id is no id.".to_owned()))?,
                content,
                comments: Vec::new(),
                device: message.cloud.map(|cloud| cloud.to_string()),
            })
        };
        self.admit_frame(id, frame).await?;
        Ok(api::MessageAnswer { id: message_id })
    }

    /// Deletes the integration's conversation `id`, as the web app's Delete
    /// does.
    pub async fn delete_integration_conversation(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        id: &ConversationId,
    ) -> Result<(), IntegrationRefusal> {
        let id = &self.integration_conversation_of(integration, id).await?.id;
        self.delete_conversation(id).await?;
        self.integration_events.forget(id);
        self.mark_integrations();
        Ok(())
    }

    /// Deletes the integration's Cloud `device`, its storage and its key,
    /// whether or not a conversation works on it.
    pub async fn delete_integration_cloud(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        device: &DeviceId,
    ) -> Result<(), IntegrationRefusal> {
        if self.integration_cloud(integration, device).await?.is_none() {
            return Err(IntegrationRefusal::NotFound("Cloud"));
        }
        // From here no conversation is made on it or moved to it.
        let _deleting = self
            .integration_deletions
            .delete(device.as_str())
            .await
            .ok_or_else(|| IntegrationRefusal::Refused("the Cloud is being deleted".to_owned()))?;
        self.delete_cloud(device).await?;
        self.mark_integrations();
        Ok(())
    }

    /// Deletes the Cloud `device`: its machine, then its device as a
    /// revocation removes one, which nothing refuses; a conversation that
    /// worked on it stays, with no Cloud until it moves to one.
    async fn delete_cloud(self: &Rc<Self>, device: &DeviceId) -> Result<(), IntegrationRefusal> {
        let control = &self.services().control;
        let Some(record) = control.device(device.clone()).await? else {
            return Ok(());
        };
        self.end_cloud_commands(device).await;
        let _held = self
            .cloud_shard()
            .delete_machine(&record)
            .await
            .map_err(|error| IntegrationRefusal::Refused(error.to_string()))?;
        self.revoke_device(device.clone()).await?;
        Ok(())
    }

    /// Delete Integration: the user's integration `id` with its
    /// conversations, its Clouds, its shared folder and its token, whose
    /// sockets close; its usage stays. False for one that is not the
    /// user's.
    pub async fn delete_integration(self: &Rc<Self>, id: &IntegrationId) -> Result<bool, IntegrationRefusal> {
        let Some(integration) = self.own_integration(id).await? else {
            return Ok(false);
        };
        let control = &self.services().control;
        // No request adds a Cloud or a conversation to it from here on, and
        // those under way have ended: what is listed below is all it holds.
        let _deleting = self
            .integration_deletions
            .delete(id.as_str())
            .await
            .ok_or_else(|| IntegrationRefusal::Refused("the integration is being deleted".to_owned()))?;
        // No program reaches it from here on.
        control.replace_integration_token(id.clone()).await?;
        self.integration_events.close(id);
        self.services().views.withdraw(id);
        for kept in control.integration_conversations(integration.id.clone()).await? {
            self.delete_conversation(&kept.conversation).await?;
        }
        for cloud in control.integration_clouds(integration.id.clone()).await? {
            self.delete_cloud(&cloud.device).await?;
        }
        self.cloud_shard()
            .delete_shared_folder(integration.id.as_str())
            .await
            .map_err(|error| IntegrationRefusal::Refused(error.to_string()))?;
        let deleted = control.delete_integration(integration.id).await?;
        self.mark_integrations();
        Ok(deleted)
    }

    /// What the agent's command `command` of the integration's conversation
    /// `id` printed, from the byte `after`, and how it ended: as
    /// `demi shell output` reads it, from its Host while it runs and from
    /// the conversation's record after.
    pub async fn integration_command_output(
        &self,
        integration: &IntegrationRecord,
        id: &ConversationId,
        command: &str,
        after: Option<u64>,
    ) -> Result<api::CommandOutput, IntegrationRefusal> {
        let id = &self.integration_conversation_of(integration, id).await?.id;
        let unknown = || IntegrationRefusal::NotFound("command");
        let found = CommandId::try_from(command).map_err(|_| unknown())?;
        let (output, end) = self
            .agent()
            .command_output(&root_of(id), &found)
            .await
            .map_err(|missed| match missed {
                CommandMissed::Unknown(_) => unknown(),
                missed @ CommandMissed::Store(..) => IntegrationRefusal::ConversationStore(missed),
                missed => IntegrationRefusal::Refused(missed.to_string()),
            })?;
        let (output, next) = output.text_after(after.unwrap_or(0));
        Ok(api::CommandOutput {
            id: command.to_owned(),
            end,
            output,
            next,
        })
    }

    /// Stops the agents of the integration's conversation `id` that `scope`
    /// names, every one without it, as the web app's stops do.
    pub async fn stop_integration_conversation(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        id: &ConversationId,
        scope: Option<api::StopScope>,
    ) -> Result<(), IntegrationRefusal> {
        let id = &self.integration_conversation_of(integration, id).await?.id;
        if scope != Some(api::StopScope::Main) {
            self.admit_frame(id, ClientFrame::AbortSubagents {}).await?;
        }
        if scope != Some(api::StopScope::Subagents) {
            self.admit_frame(id, ClientFrame::Abort {}).await?;
        }
        Ok(())
    }

    /// Hands the integration's conversation `id` one of the web app's
    /// frames that act on its turn, its queue or a command, as a page's
    /// control does. A frame that names a subagent the conversation does
    /// not have is refused, which the session would take without a word.
    pub async fn act_on_integration_conversation(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        id: &ConversationId,
        frame: ClientFrame,
    ) -> Result<(), IntegrationRefusal> {
        let id = &self.integration_conversation_of(integration, id).await?.id;
        let named = match &frame {
            ClientFrame::AbortSubagent { subagent_id } => Some(subagent_id),
            ClientFrame::Compact { node } => node.as_ref(),
            _ => None,
        };
        if let Some(node) = named {
            let (_, entries) = self
                .conversation_agents(id)
                .await
                .map_err(|error| IntegrationRefusal::Refused(error.to_string()))?;
            let parent = entries.iter().find_map(|entry| match &entry.role {
                AgentRole::Subagent(child) if &entry.id == node => Some(&child.parent),
                _ => None,
            });
            let Some(parent) = parent else {
                return Err(IntegrationRefusal::NotFound("agent"));
            };
            // The page's Stop on a subagent stops one of the main agent's
            // own, never a subagent's subagent.
            if matches!(frame, ClientFrame::AbortSubagent { .. }) && parent.as_str() != id.as_str() {
                return Err(IntegrationRefusal::Refused(
                    "only the main agent's own subagents stop this way".to_owned(),
                ));
            }
        }
        self.admit_frame(id, frame).await?;
        Ok(())
    }

    /// Acts on the message `message` of the integration's conversation `id`
    /// that waits, queued or as a steer the turn has not taken: `queued`
    /// makes the frame for a queued one, `steered` for a steer, as the
    /// page's control on either row does. The main agent's entry says which
    /// it is, since a session answers a frame for an id it does not hold
    /// with nothing.
    pub async fn act_on_integration_message(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        id: &ConversationId,
        message: String,
        queued: fn(TurnId) -> ClientFrame,
        steered: fn(BlockId) -> ClientFrame,
    ) -> Result<(), IntegrationRefusal> {
        let id = &self.integration_conversation_of(integration, id).await?.id;
        let (_, entries) = self
            .conversation_agents(id)
            .await
            .map_err(|error| IntegrationRefusal::Refused(error.to_string()))?;
        let root = entries.into_iter().find(|entry| matches!(entry.role, AgentRole::Root));
        let frame = root.and_then(|root| {
            if let Some(waiting) = root.queue.into_iter().find(|waiting| waiting.id.as_str() == message) {
                return Some(queued(waiting.id));
            }
            let steer = root.pending_steers.into_iter().find(|steer| steer.id.as_str() == message)?;
            Some(steered(steer.id))
        });
        let frame = frame.ok_or(IntegrationRefusal::NotFound("waiting message"))?;
        // A message that left the queue since it was read is gone.
        self.admit_frame(id, frame).await.map_err(|refusal| {
            if refusal.code == ErrorCode::NotFound.to_string() {
                IntegrationRefusal::NotFound("waiting message")
            } else {
                refusal.into()
            }
        })
    }

    /// Hands the subagent `agent` of the integration's conversation `id` a
    /// message, as its user's.
    pub async fn message_integration_subagent(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        id: &ConversationId,
        agent: &NodeId,
        message: api::SubagentMessage,
    ) -> Result<(), IntegrationRefusal> {
        let id = &self.integration_conversation_of(integration, id).await?.id;
        let (_, entries) = self
            .conversation_agents(id)
            .await
            .map_err(|error| IntegrationRefusal::Refused(error.to_string()))?;
        let held = entries
            .iter()
            .any(|entry| &entry.id == agent && matches!(entry.role, AgentRole::Subagent(_)));
        if !held {
            return Err(IntegrationRefusal::NotFound("subagent"));
        }
        let block = BlockId::try_from(uuid::Uuid::new_v4().simple().to_string()).expect("a UUID is a block id");
        let root = root_of(id);
        let text = message.text;
        self.with_open_tree(id, async || {
            self.agent()
                .message_subagent(&root, agent, block, text)
                .await
                .map_err(|reason| Refusal::new(ErrorCode::SendFailed, reason))
        })
        .await?;
        Ok(())
    }

    /// Forks the integration's conversation `id` through `request.block_id`
    /// into a new conversation of the integration's, with the source's
    /// workflow, identity and commands and the key the request names; or
    /// finds the one of that key, which the flag says.
    pub async fn fork_integration_conversation(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        id: &ConversationId,
        request: api::ForkRequest,
    ) -> Result<(api::Conversation, bool), IntegrationRefusal> {
        let control = &self.services().control;
        let id = self.integration_conversation_of(integration, id).await?.id;
        let source = control
            .integration_conversation(id.clone())
            .await?
            .ok_or(IntegrationRefusal::NotFound("conversation"))?;
        if let Some(key) = &request.key
            && let Some(existing) = control.integration_conversation_by_key(integration.id.clone(), key.clone()).await?
        {
            return Ok((self.integration_conversation_of(integration, &existing).await?, false));
        }
        let destination = ConversationId::try_from(uuid::Uuid::new_v4().to_string().as_str()).expect("a UUID is a conversation id");
        let forked = self.fork(id, destination, request.block_id).await.map_err(|refusal| match refusal {
            ForkRefusal::SourceNotFound => IntegrationRefusal::NotFound("conversation"),
            ForkRefusal::Target(reason) => IntegrationRefusal::Invalid(reason),
            ForkRefusal::Storage(error) => IntegrationRefusal::Storage(error),
            refusal => IntegrationRefusal::Refused(refusal.to_string()),
        })?;
        let kept = IntegrationConversation {
            conversation: forked.record.id.clone(),
            integration: integration.id.clone(),
            key: request.key.clone(),
            workflow: source.workflow,
            identity: source.identity,
            commands: source.commands,
            browser_profile: source.browser_profile,
        };
        if let Some(first) = control.record_integration_conversation(kept).await? {
            let _ = self.delete_conversation(&forked.record.id).await;
            return Ok((self.integration_conversation_of(integration, &first).await?, false));
        }
        self.mark_integrations();
        Ok((self.integration_conversation_shown(forked.record, request.key).await?, true))
    }

    /// The integration's conversation `id`, as the integration sees it; not
    /// found for one of another integration's or of the user's own.
    pub async fn integration_conversation_of(
        &self,
        integration: &IntegrationRecord,
        id: &ConversationId,
    ) -> Result<api::Conversation, IntegrationRefusal> {
        let control = &self.services().control;
        let kept = control
            .integration_conversation(id.clone())
            .await?
            .filter(|kept| kept.integration == integration.id)
            .ok_or(IntegrationRefusal::NotFound("conversation"))?;
        let record = control
            .conversation(id.clone())
            .await?
            .ok_or(IntegrationRefusal::NotFound("conversation"))?;
        Ok(self.integration_conversation_shown(record, kept.key).await?)
    }

    async fn integration_conversation_shown(
        &self,
        record: ConversationRecord,
        key: Option<String>,
    ) -> Result<api::Conversation, StorageError> {
        // The device it works on, whether its directory is a project's or
        // the Cloud's own.
        let (cloud, directory) = self.integration_place(&record.id).await?;
        let (summary, facts) = self.conversation_summary_with_facts(record).await?;
        let last_turn = facts.last_turn.map(|turn| api::LastTurn {
            id: turn.turn,
            cause: turn.cause,
            outcome: outcome_of(turn.outcome),
            ended_by: turn.id,
            answer: turn.answer,
        });
        Ok(api::Conversation {
            id: summary.id,
            key,
            cloud,
            directory,
            title: summary.title.unwrap_or_default(),
            status: summary.status,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
            revision: facts.revision,
            last_turn,
        })
    }
}

/// An agent entry as an integration sees it; none for a side question,
/// which is the web app's.
fn api_agent(entry: AgentEntry) -> Option<api::Agent> {
    let queue = entry
        .queue
        .iter()
        .map(|message| api::Waiting {
            id: message.id.to_string(),
            text: content_text(&message.content),
            cloud: message.device.as_ref().and_then(|device| DeviceId::try_from(device.clone()).ok()),
        })
        .collect();
    let pending_steers = entry
        .pending_steers
        .iter()
        .map(|steer| api::Waiting {
            id: steer.id.to_string(),
            text: content_text(&steer.content),
            cloud: None,
        })
        .collect();
    match entry.role {
        AgentRole::Root => Some(api::Agent {
            id: entry.id,
            number: 0,
            parent: None,
            description: String::new(),
            phase: entry.phase,
            cloud: None,
            round: None,
            queue,
            pending_steers,
        }),
        AgentRole::Subagent(child) => Some(api::Agent {
            id: entry.id,
            number: child.number,
            parent: Some(child.parent),
            description: child.description,
            phase: entry.phase,
            cloud: child.device.and_then(|device| DeviceId::try_from(device).ok()),
            round: Some(api::Round {
                number: child.round,
                profile: child.profile,
                model: child.model,
                state: child.state,
                started_at: child.started_at,
                ended_at: child.ended_at,
            }),
            queue,
            pending_steers,
        }),
        AgentRole::Side(_) => None,
    }
}

/// The text of a message's content, its text parts joined.
fn content_text(content: &[UserContentBlock]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            UserContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The sockets of the user's integrations (`GET /v1/socket`), each of which
/// hears every change of its integration's conversations and Clouds, and the
/// one that serves an integration's plugin (`crate::integration_plugins`).
#[derive(Default)]
pub struct IntegrationEvents {
    sockets: RefCell<HashMap<IntegrationId, Vec<Socket>>>,
    next_socket: Cell<u64>,
    /// The integration of each conversation whose tree an observer hears.
    observed: RefCell<HashMap<ConversationId, IntegrationId>>,
    /// Told whenever a socket declares, which a call that waits for one
    /// awaits.
    pub(crate) declared: Notify,
    /// The plugin requests that wait for their replies, by id.
    pub(crate) calls: RefCell<HashMap<u64, crate::integration_plugins::Call>>,
    /// The last call's id (`crate::integration_plugins`).
    pub(crate) next_call: Cell<u64>,
    /// Each conversation as its sockets last heard it, which a change that
    /// shows nothing new is not told again against.
    shown: RefCell<HashMap<ConversationId, api::Conversation>>,
    /// The conversations whose read for their sockets is under way: a
    /// change meanwhile is in what it reads.
    reading: RefCell<std::collections::HashSet<ConversationId>>,
    /// The Clouds' state changes not told yet, in the order they came, and
    /// whether a task tells them: each needs a read of the Cloud's
    /// integration, and one task reading them in turn keeps their order.
    cloud_changes: RefCell<std::collections::VecDeque<(DeviceId, CloudState)>>,
    telling_clouds: Cell<bool>,
}

/// One socket of an integration's.
struct Socket {
    id: u64,
    /// What happens to the integration's conversations, at most
    /// [`SOCKET_BACKLOG`] unread.
    events: mpsc::Sender<api::Event>,
    /// The plugin's requests, cancellations and answers, which never wait
    /// behind events nor are dropped: as many as the calls its agents make
    /// at once.
    control: mpsc::UnboundedSender<api::Event>,
    /// Whether events were dropped since the program last read up.
    behind: Arc<AtomicBool>,
    /// Whether it declared the integration's plugin, so its requests may go
    /// to it.
    serves: bool,
}

/// How many events a socket may hold that its program has not read: a
/// program that stops reading would otherwise hold the backend's memory
/// without end. Past it the socket's events are dropped and it is told it
/// fell behind, after which its program reconciles from the state.
const SOCKET_BACKLOG: usize = 4096;

/// What a socket of an integration's reads: its events, its plugin's
/// requests, and whether it fell behind.
pub struct IntegrationSocket {
    pub id: u64,
    pub events: mpsc::Receiver<api::Event>,
    pub control: mpsc::UnboundedReceiver<api::Event>,
    pub behind: Arc<AtomicBool>,
}

impl IntegrationEvents {
    /// A socket of `integration`'s from now on, by its number, until its
    /// receivers go.
    pub fn subscribe(&self, integration: IntegrationId) -> IntegrationSocket {
        let (events, events_received) = mpsc::channel(SOCKET_BACKLOG);
        let (control, control_received) = mpsc::unbounded_channel();
        let behind = Arc::new(AtomicBool::new(false));
        let id = self.next_socket.get() + 1;
        self.next_socket.set(id);
        self.sockets.borrow_mut().entry(integration).or_default().push(Socket {
            id,
            events,
            control,
            behind: behind.clone(),
            serves: false,
        });
        IntegrationSocket {
            id,
            events: events_received,
            control: control_received,
            behind,
        }
    }

    /// Tells every socket of `integration`'s; a socket whose receiver went
    /// is dropped, and one that is full is marked behind.
    pub(crate) fn publish(&self, integration: &IntegrationId, event: api::Event) {
        let mut sockets = self.sockets.borrow_mut();
        let Some(list) = sockets.get_mut(integration) else {
            return;
        };
        list.retain(|socket| match socket.events.try_send(event.clone()) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                socket.behind.store(true, Ordering::Release);
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        });
        if list.is_empty() {
            sockets.remove(integration);
        }
    }

    /// Tells the socket `socket` of `integration`'s alone, on its plugin's
    /// queue; false when it is gone, which drops it.
    pub(crate) fn send_to(&self, integration: &IntegrationId, socket: u64, event: api::Event) -> bool {
        let mut sockets = self.sockets.borrow_mut();
        let Some(list) = sockets.get_mut(integration) else {
            return false;
        };
        let Some(index) = list.iter().position(|candidate| candidate.id == socket) else {
            return false;
        };
        if list[index].control.send(event).is_ok() {
            return true;
        }
        list.remove(index);
        false
    }

    /// Marks the socket `socket` of `integration`'s as the plugin's server,
    /// moved last, where [`Self::server`] looks first.
    pub(crate) fn serve(&self, integration: &IntegrationId, socket: u64) {
        if let Some(list) = self.sockets.borrow_mut().get_mut(integration)
            && let Some(index) = list.iter().position(|candidate| candidate.id == socket)
        {
            let mut declared = list.remove(index);
            declared.serves = true;
            list.push(declared);
        }
        self.declared.notify_waiters();
        self.resend(integration);
    }

    /// A socket of `integration`'s that serves its plugin, the one that
    /// declared last.
    pub(crate) fn server(&self, integration: &IntegrationId) -> Option<u64> {
        self.sockets
            .borrow()
            .get(integration)?
            .iter()
            .rev()
            .find(|socket| socket.serves && !socket.control.is_closed())
            .map(|socket| socket.id)
    }

    /// The socket `socket` of `integration`'s went: its requests wait for
    /// the next socket that serves the plugin, which may be one that does
    /// now.
    pub(crate) fn closed(&self, integration: &IntegrationId, socket: u64) {
        if let Some(list) = self.sockets.borrow_mut().get_mut(integration) {
            list.retain(|candidate| candidate.id != socket);
        }
        for call in self.calls.borrow_mut().values_mut() {
            if call.integration == *integration && call.socket == Some(socket) {
                call.socket = None;
            }
        }
        self.resend(integration);
    }

    /// Sends `socket` the answer to the `port` message `port` of the call
    /// `id`. Input that does not reach it, since the socket closed, waits
    /// on the call for its next `read_stdin`: it was read from the caller
    /// once and cannot be read again.
    pub(crate) fn answer(&self, integration: &IntegrationId, socket: u64, id: u64, port: u64, response: PortResponse) {
        let input = match &response {
            PortResponse::Input { bytes: Some(bytes) } => Some(bytes.clone()),
            _ => None,
        };
        if self.send_to(integration, socket, api::Event::Answer { id, port, response }) {
            return;
        }
        if let Some(bytes) = input
            && let Some(call) = self.calls.borrow_mut().get_mut(&id)
        {
            call.unread = Some(match call.unread.take() {
                Some(earlier) => B64Bytes::from([earlier.as_bytes(), bytes.as_bytes()].concat()),
                None => bytes,
            });
        }
    }

    /// The call `id` lost its socket: it waits for the next.
    pub(crate) fn orphan(&self, integration: &IntegrationId, id: u64) {
        if let Some(call) = self.calls.borrow_mut().get_mut(&id) {
            call.socket = None;
        }
        self.resend(integration);
    }

    /// Sends each of `integration`'s calls that lost their socket to the one
    /// that serves its plugin, if any, again under its id: its program makes
    /// its work idempotent by it.
    fn resend(&self, integration: &IntegrationId) {
        loop {
            let Some(socket) = self.server(integration) else {
                return;
            };
            let waiting: Vec<(u64, api::PluginRequest)> = self
                .calls
                .borrow()
                .iter()
                .filter(|(_, call)| call.integration == *integration && call.socket.is_none())
                .map(|(id, call)| (*id, call.request.clone()))
                .collect();
            let mut lost = false;
            for (id, request) in waiting {
                if !self.send_to(integration, socket, api::Event::Request { id, request, resent: true }) {
                    // That socket closed too: the next one serving gets them.
                    lost = true;
                    break;
                }
                if let Some(call) = self.calls.borrow_mut().get_mut(&id) {
                    call.socket = Some(socket);
                }
            }
            if !lost {
                return;
            }
        }
    }

    /// Ends every socket of `integration`'s, as a new token does.
    fn close(&self, integration: &IntegrationId) {
        self.sockets.borrow_mut().remove(integration);
    }

    /// Ends every socket of every integration of the user's, as disabling
    /// the account does.
    pub(crate) fn close_all(&self) {
        self.sockets.borrow_mut().clear();
    }

    fn heard(&self, integration: &IntegrationId) -> bool {
        self.sockets.borrow().contains_key(integration)
    }

    /// Tells `integration`'s sockets the conversation `shown`, unless they
    /// last heard it with the same status, latest ended turn and title.
    fn tell(&self, integration: &IntegrationId, shown: api::Conversation) {
        let same = |heard: &api::Conversation| {
            heard.status == shown.status && heard.last_turn == shown.last_turn && heard.title == shown.title
        };
        if self.shown.borrow().get(&shown.id).is_some_and(same) {
            return;
        }
        self.shown.borrow_mut().insert(shown.id.clone(), shown.clone());
        self.publish(integration, api::Event::Conversation { conversation: shown });
    }

    /// Forgets what the sockets heard of the conversation `id`, which was
    /// deleted or closed.
    pub(crate) fn forget(&self, id: &ConversationId) {
        self.shown.borrow_mut().remove(id);
    }
}

/// Observes the trees of the integrations' conversations: each frame of one
/// becomes the events its integration's sockets hear.
pub(crate) struct IntegrationObservers {
    pub(crate) shard: Weak<Shard>,
}

impl TreeObservers for IntegrationObservers {
    fn observer<'a>(&'a self, root: &'a NodeId) -> LocalBoxFuture<'a, Option<Observer>> {
        Box::pin(async move {
            let shard = self.shard.upgrade()?;
            let conversation = conversation_of(root);
            let kept = shard.services().control.integration_conversation(conversation.clone()).await.ok()??;
            let weak = self.shard.clone();
            let integration = kept.integration;
            shard
                .integration_events
                .observed
                .borrow_mut()
                .insert(conversation.clone(), integration.clone());
            Some(Rc::new(move |frame: &ServerFrame| {
                let Some(shard) = weak.upgrade() else {
                    return;
                };
                for event in events_of(&conversation, frame) {
                    shard.integration_events.publish(&integration, event);
                }
            }) as Observer)
        })
    }
}

/// The events a tree's frame of the conversation `conversation` is to an
/// integration: its transcripts' patches and its agents' changes.
fn events_of(conversation: &ConversationId, frame: &ServerFrame) -> Vec<api::Event> {
    match frame {
        ServerFrame::TranscriptPatch { node, patches, version, .. } => vec![api::Event::Transcript {
            conversation: conversation.clone(),
            agent: node.clone(),
            version: version.clone(),
            patches: patches.clone(),
        }],
        ServerFrame::Agents { agents, .. } => agents
            .iter()
            .cloned()
            .filter_map(api_agent)
            .map(|agent| api::Event::Agent {
                conversation: conversation.clone(),
                agent,
            })
            .collect(),
        ServerFrame::Agent { agent, .. } => api_agent(agent.clone())
            .map(|agent| api::Event::Agent {
                conversation: conversation.clone(),
                agent,
            })
            .into_iter()
            .collect(),
        ServerFrame::RetryScheduled { attempt, delay_ms, code, diagnostics } => vec![api::Event::RetryScheduled {
            conversation: conversation.clone(),
            attempt: *attempt,
            delay_ms: *delay_ms,
            code: code.clone(),
            diagnostics: diagnostics.clone(),
        }],
        _ => Vec::new(),
    }
}

impl Shard {
    /// The workflow of the integration's conversation `id`, its YAML, and
    /// whether its integration has subagents on: none for a conversation of
    /// the user's own; a workflow of none for one made without a workflow.
    pub(crate) async fn integration_workflow(
        &self,
        id: &ConversationId,
    ) -> Result<Option<(Option<String>, bool)>, String> {
        let control = &self.services().control;
        let Some(kept) = control.integration_conversation(id.clone()).await.map_err(|error| error.to_string())? else {
            return Ok(None);
        };
        let subagents = control
            .integration(kept.integration)
            .await
            .map_err(|error| error.to_string())?
            .is_some_and(|record| record.capabilities.subagents);
        Ok(Some((kept.workflow, subagents)))
    }

    /// The capabilities of the integration whose Cloud `device` is; none for
    /// a device of the user's own.
    pub(crate) async fn cloud_capabilities(
        &self,
        device: &DeviceId,
    ) -> Result<Option<demi_shared_types::IntegrationCapabilities>, String> {
        let control = &self.services().control;
        let Some(cloud) = control.integration_cloud(device.clone()).await.map_err(|error| error.to_string())? else {
            return Ok(None);
        };
        Ok(control
            .integration(cloud.integration)
            .await
            .map_err(|error| error.to_string())?
            .map(|record| record.capabilities))
    }

    /// The capabilities of the integration whose conversation `root` is, read
    /// now; none for no root and for a conversation of the user's own.
    pub(crate) async fn integration_capabilities(
        &self,
        root: Option<&NodeId>,
    ) -> Result<Option<demi_shared_types::IntegrationCapabilities>, String> {
        let Some(root) = root else {
            return Ok(None);
        };
        let control = &self.services().control;
        let Some(kept) = control
            .integration_conversation(conversation_of(root))
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        Ok(control
            .integration(kept.integration)
            .await
            .map_err(|error| error.to_string())?
            .map(|record| record.capabilities))
    }

    /// The integration of the conversation `conversation`, while an observer
    /// hears its tree.
    pub(crate) fn integration_of_open(&self, conversation: &ConversationId) -> Option<IntegrationId> {
        self.integration_events.observed.borrow().get(conversation).cloned()
    }

    /// The socket `socket` of the integration `integration`'s went.
    pub fn integration_socket_closed(&self, integration: &IntegrationId, socket: u64) {
        self.integration_events.closed(integration, socket);
    }

    /// A socket of the integration `integration`'s from now on, until its
    /// receivers go.
    pub fn subscribe_integration(&self, integration: IntegrationId) -> IntegrationSocket {
        self.integration_events.subscribe(integration)
    }

    /// The conversation's status may have changed: an integration's sockets
    /// hear the conversation now. A tree that starts working says `running`
    /// in the step it starts, before anything it does, with the rest as its
    /// sockets last heard it; the rest is read after, such as a turn that
    /// ended while a subagent works on.
    pub(crate) fn integration_status_changed(self: &Rc<Self>, conversation: &ConversationId) {
        if self.agent().tree(&root_of(conversation)).is_some_and(|tree| tree.works())
            && let Some(integration) = self.integration_of_open(conversation)
        {
            let shown = self.integration_events.shown.borrow().get(conversation).cloned();
            if let Some(mut shown) = shown
                && !matches!(shown.status, ConversationStatus::Running | ConversationStatus::Compacting)
            {
                shown.status = ConversationStatus::Running;
                self.integration_events.tell(&integration, shown);
            }
        }
        // A tree that closed has no observer any more.
        if self.agent().tree(&root_of(conversation)).is_none() {
            self.integration_events.observed.borrow_mut().remove(conversation);
        }
        self.integration_conversation_changed(conversation);
    }

    /// The Cloud `device` changed state: its integration's sockets hear it,
    /// after every change before it. The user's own Cloud is of none.
    pub(crate) fn integration_cloud_changed(self: &Rc<Self>, device: &DeviceId, state: CloudState) {
        let events = &self.integration_events;
        if events.sockets.borrow().is_empty() {
            return;
        }
        events.cloud_changes.borrow_mut().push_back((device.clone(), state));
        if events.telling_clouds.replace(true) {
            return;
        }
        let shard = self.clone();
        self.tasks().spawn_local(async move {
            loop {
                let next = shard.integration_events.cloud_changes.borrow_mut().pop_front();
                let Some((device, state)) = next else {
                    break;
                };
                if let Ok(Some(cloud)) = shard.services().control.integration_cloud(device.clone()).await {
                    shard
                        .integration_events
                        .publish(&cloud.integration, api::Event::Cloud { cloud: device, state });
                }
            }
            shard.integration_events.telling_clouds.set(false);
        });
    }

    /// Something an integration's conversation shows may have changed, its
    /// status, its latest ended turn or its title: its sockets hear the
    /// conversation, read now, when it shows something new. Changes that
    /// come while a read waits to start are in what it reads.
    pub(crate) fn integration_conversation_changed(self: &Rc<Self>, conversation: &ConversationId) {
        // No socket of the user's integrations hears anything: a user of the
        // web app alone pays nothing for this.
        if self.integration_events.sockets.borrow().is_empty() {
            return;
        }
        if !self.integration_events.reading.borrow_mut().insert(conversation.clone()) {
            return;
        }
        let shard = self.clone();
        let conversation = conversation.clone();
        self.tasks().spawn_local(async move {
            shard.integration_events.reading.borrow_mut().remove(&conversation);
            let control = &shard.services().control;
            let Ok(Some(kept)) = control.integration_conversation(conversation.clone()).await else {
                shard.integration_events.forget(&conversation);
                return;
            };
            if !shard.integration_events.heard(&kept.integration) {
                return;
            }
            let Ok(Some(record)) = control.conversation(conversation.clone()).await else {
                return;
            };
            if let Ok(shown) = shard.integration_conversation_shown(record, kept.key).await {
                // A turn that started meanwhile told its own status in its
                // step; this one's would come after it, and be older. The
                // status is the sidebar's, which a tree's running reads.
                let tree = shard.agent().tree(&root_of(&conversation));
                let running = tree.as_ref().is_some_and(|tree| tree.is_running());
                if running != matches!(shown.status, ConversationStatus::Running | ConversationStatus::Compacting) {
                    return;
                }
                shard.integration_events.tell(&kept.integration, shown);
                // What a closed conversation's sockets heard is read again
                // when it opens: it need not be kept.
                if tree.is_none() {
                    shard.integration_events.forget(&conversation);
                }
            }
        });
    }
}

impl Shard {
    /// Whose work a call of the conversation's agent numbered `agent`, in
    /// its turn `turn`, is: the device it works on now, the integration whose
    /// conversation it is, and the program's message it descends from.
    pub(crate) async fn attribution(&self, conversation: &ConversationId, agent: Option<u64>, turn: Option<&str>) -> Attribution {
        let control = &self.services().control;
        let integration = control
            .integration_conversation(conversation.clone())
            .await
            .ok()
            .flatten()
            .map(|kept| kept.integration);
        let device = match agent {
            Some(number) => self.agent_device(conversation, number).await,
            None => None,
        };
        // Only an integration's program has messages its work descends from.
        let message_id = match (&integration, agent, turn.and_then(|turn| TurnId::try_from(turn.to_owned()).ok())) {
            (Some(_), Some(number), Some(turn)) => match self.agent_node(conversation, number) {
                Some(node) => self.turn_message(conversation, &node, &turn).await,
                None => None,
            },
            _ => None,
        };
        Attribution {
            device,
            integration,
            message_id,
        }
    }

    /// The live node of the conversation's agent numbered `number`; none
    /// when the conversation is not open.
    fn agent_node(&self, conversation: &ConversationId, number: u64) -> Option<NodeId> {
        self.agent()
            .tree(&root_of(conversation))?
            .nodes()
            .into_iter()
            .map(|node| node.record())
            .find(|record| record.number == number)
            .map(|record| record.id)
    }

    /// The device the conversation's agent numbered `number` works on: its
    /// own Host, else the conversation's.
    async fn agent_device(&self, conversation: &ConversationId, number: u64) -> Option<DeviceId> {
        let own = self.agent().tree(&root_of(conversation)).and_then(|tree| {
            tree.nodes()
                .into_iter()
                .map(|node| node.record())
                .find(|record| record.number == number)
                .and_then(|record| record.placement)
                .and_then(|placement| placement.device)
        });
        if let Some(device) = own {
            return DeviceId::try_from(device).ok();
        }
        let control = &self.services().control;
        let record = control.conversation(conversation.clone()).await.ok()??;
        let target = self.host_shard().resolve_target(&record).await.ok()?;
        match target.device() {
            Some(device) => Some(device.clone()),
            // The owner's own Cloud.
            None => control.managed_device(record.owner.clone()).await.ok()?.map(|device| device.id),
        }
    }
}

/// What an integration's deletions and the requests that add to it agree
/// on, by key, a Cloud's or an integration's: a request that makes a
/// conversation or a Cloud, or moves a conversation, holds a lease of what
/// it adds to from its check to its write; a deletion refuses new leases,
/// then waits for those held, so it sees every addition that passed its
/// check.
#[derive(Default)]
pub(crate) struct Deletions {
    keys: RefCell<HashMap<String, Held>>,
    released: Notify,
}

#[derive(Default)]
struct Held {
    leases: u32,
    deleting: bool,
}

/// A request's lease of the keys it adds to.
pub(crate) struct Lease<'a> {
    deletions: &'a Deletions,
    keys: Vec<String>,
}

/// A deletion's hold of its key; new leases are refused until it drops.
pub(crate) struct Deleting<'a> {
    deletions: &'a Deletions,
    key: String,
}

impl Deletions {
    /// A lease of `keys`, none while one of them is being deleted.
    pub(crate) fn lease(&self, keys: &[&str]) -> Option<Lease<'_>> {
        let mut held = self.keys.borrow_mut();
        if keys.iter().any(|key| held.get(*key).is_some_and(|held| held.deleting)) {
            return None;
        }
        for key in keys {
            held.entry((*key).to_owned()).or_default().leases += 1;
        }
        Some(Lease {
            deletions: self,
            keys: keys.iter().map(|key| (*key).to_owned()).collect(),
        })
    }

    /// Starts deleting `key`: new leases of it are refused, and this waits
    /// until the held ones end. None while another deletion of it runs.
    pub(crate) async fn delete(&self, key: &str) -> Option<Deleting<'_>> {
        {
            let mut held = self.keys.borrow_mut();
            let entry = held.entry(key.to_owned()).or_default();
            if entry.deleting {
                return None;
            }
            entry.deleting = true;
        }
        let deleting = Deleting {
            deletions: self,
            key: key.to_owned(),
        };
        loop {
            let released = self.released.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            if self.keys.borrow().get(key).is_none_or(|held| held.leases == 0) {
                return Some(deleting);
            }
            released.await;
        }
    }

    fn forget_if_unused(held: &mut HashMap<String, Held>, key: &str) {
        if held.get(key).is_some_and(|held| held.leases == 0 && !held.deleting) {
            held.remove(key);
        }
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let mut held = self.deletions.keys.borrow_mut();
        for key in &self.keys {
            if let Some(entry) = held.get_mut(key) {
                entry.leases -= 1;
            }
            Deletions::forget_if_unused(&mut held, key);
        }
        drop(held);
        self.deletions.released.notify_waiters();
    }
}

impl Drop for Deleting<'_> {
    fn drop(&mut self) {
        let mut held = self.deletions.keys.borrow_mut();
        if let Some(entry) = held.get_mut(&self.key) {
            entry.deleting = false;
        }
        Deletions::forget_if_unused(&mut held, &self.key);
    }
}
