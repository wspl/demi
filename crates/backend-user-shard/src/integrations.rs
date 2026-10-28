//! A user's integrations (`crates/backend-database/src/integrations.rs`): the
//! programs that build an agent product on Demi through its API, which the
//! user makes and changes in Settings › Integrations.
//!
//! An integration infers with every provider its user has, and its
//! conversations are in a workspace of its own that no page lists.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use demi_agent_server::{Observer, TreeObservers};
use demi_backend_host_access::{conversation_of, root_of};
use demi_conversation_lane_protocol::ServerFrame;
use demi_shared_types::NodeId;
use futures_util::future::LocalBoxFuture;
use tokio::sync::{Notify, mpsc};

use demi_backend_database::StorageError;
use demi_backend_database::conversation_index::{ConversationRecord, ConversationStart, Creation, RecordChange};
use demi_backend_database::integrations::{
    IntegrationCloud, IntegrationConversation, IntegrationPatch as StoredPatch, IntegrationRecord,
};
use demi_backend_host_access::transition::ChangeRefusal;
use demi_conversation_lane_protocol::{AgentEntry, AgentRole, ClientContent, ClientFrame, Refusal, SendMessage};
use demi_integration_api_protocol as api;
use demi_shared_types::{BlockId, SessionPhase, TurnId, WorkflowChoice};
use demi_web_api_protocol::conversations::{ConversationStatus, ConversationTarget, ModelSettings};

use demi_web_api_protocol::ids::{ConversationId, DeviceId, IntegrationId, OperationId, ProviderId};
use demi_web_api_protocol::integrations::{IntegrationDto, IntegrationPatch, IntegrationWithToken};
use tokio_util::sync::CancellationToken;

use crate::shard::Shard;

impl Shard {
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
    ) -> Result<Option<IntegrationDto>, StorageError> {
        if self.own_integration(&id).await?.is_none() {
            return Ok(None);
        }
        let stored = StoredPatch {
            name: patch.name.map(|name| name.into_string()),
            capabilities: patch.capabilities,
        };
        match self.services().control.update_integration(id, stored).await? {
            Some(record) => Ok(Some(self.integration_dto(record).await?)),
            None => Ok(None),
        }
    }

    /// Replace Token: a new token for the user's integration `id`, which the
    /// old one no longer opens; none for one that is not the user's.
    pub async fn replace_integration_token(&self, id: IntegrationId) -> Result<Option<IntegrationWithToken>, StorageError> {
        let Some(record) = self.own_integration(&id).await? else {
            return Ok(None);
        };
        let Some(token) = self.services().control.replace_integration_token(id.clone()).await? else {
            return Ok(None);
        };
        // The sockets the old token opened end with it.
        self.integration_events.close(&id);
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
            capabilities: record.capabilities,
            created_at: record.created_at,
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
    #[error(transparent)]
    Storage(#[from] StorageError),
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
    /// has it. It boots at its first use.
    pub async fn create_integration_cloud(
        &self,
        integration: &IntegrationRecord,
        key: Option<String>,
    ) -> Result<api::Cloud, IntegrationRefusal> {
        let cloud = self
            .services()
            .control
            .create_integration_cloud(integration.id.clone(), key)
            .await?;
        self.integration_cloud_shown(cloud)
            .await?
            .ok_or(IntegrationRefusal::NotFound("Cloud"))
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
        self.cloud_shard()
            .reset_machine(&record, operation)
            .await
            .map_err(|error| IntegrationRefusal::Refused(error.to_string()))?;
        self.integration_cloud(integration, device)
            .await?
            .ok_or(IntegrationRefusal::NotFound("Cloud"))
    }

    async fn integration_cloud_shown(&self, cloud: IntegrationCloud) -> Result<Option<api::Cloud>, StorageError> {
        let Some(device) = self.services().control.device(cloud.device.clone()).await? else {
            return Ok(None);
        };
        let state = self.cloud_shard().machine_state(&device).await?;
        Ok(Some(api::Cloud {
            id: cloud.device,
            key: cloud.key,
            state,
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

    /// Makes a conversation of the integration's on its Cloud `request.cloud`,
    /// or with a key answers the one that has it, as it is.
    pub async fn create_integration_conversation(
        &self,
        integration: &IntegrationRecord,
        request: api::CreateConversation,
    ) -> Result<api::Conversation, IntegrationRefusal> {
        let control = &self.services().control;
        if let Some(key) = &request.key
            && let Some(id) = control.integration_conversation_by_key(integration.id.clone(), key.clone()).await?
        {
            return self.integration_conversation_of(integration, &id).await;
        }
        if self.integration_cloud(integration, &request.cloud).await?.is_none() {
            return Err(IntegrationRefusal::NotFound("Cloud"));
        }
        let model = self
            .chosen_selection(&ModelSettings {
                provider_id: ProviderId::try_from(request.model.provider_id.clone())
                    .map_err(|_| IntegrationRefusal::Refused("No provider has that id.".to_owned()))?,
                model_id: request.model.model_id.clone(),
                thinking_effort: None,
                service_tier_id: None,
            })
            .await?;
        let id = ConversationId::try_from(uuid::Uuid::new_v4().to_string().as_str()).expect("a UUID is a conversation id");
        let start = ConversationStart {
            workspace: integration.workspace.clone(),
            title: None,
            pinned: false,
            target: ConversationTarget::Device {
                device_id: request.cloud.clone(),
                path: CONVERSATION_DIRECTORY.to_owned(),
            },
            model: Some(model),
            workflow: WorkflowChoice::None,
        };
        let record = match self.create_conversation_from(id.clone(), start).await? {
            Creation::Created(record) | Creation::Existing(record) => record,
            Creation::Unavailable => return Err(IntegrationRefusal::Refused("The conversation id is taken.".to_owned())),
            Creation::Unselectable(reason) => return Err(ChangeRefusal::from(reason).into()),
        };
        let kept = IntegrationConversation {
            conversation: record.id.clone(),
            integration: integration.id.clone(),
            key: request.key.clone(),
            workflow: request.workflow.clone(),
        };
        // Two requests with one key made two conversations: the first kept
        // is the key's, and this one goes.
        if let Some(first) = control.record_integration_conversation(kept).await? {
            let _ = self.delete_conversation(&record.id).await;
            return self.integration_conversation_of(integration, &first).await;
        }
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

    /// Moves the integration's conversation `id` to another of its Clouds, or
    /// renames it.
    pub async fn patch_integration_conversation(
        &self,
        integration: &IntegrationRecord,
        id: &ConversationId,
        patch: api::ConversationPatch,
    ) -> Result<api::Conversation, IntegrationRefusal> {
        let id = &self.integration_conversation_of(integration, id).await?.id;
        if let Some(cloud) = patch.cloud {
            if self.integration_cloud(integration, &cloud).await?.is_none() {
                return Err(IntegrationRefusal::NotFound("Cloud"));
            }
            let target = ConversationTarget::Device {
                device_id: cloud,
                path: CONVERSATION_DIRECTORY.to_owned(),
            };
            self.move_to(id, target, &CancellationToken::new()).await?;
        }
        if let Some(title) = patch.title {
            self.transition(id, RecordChange::Title(title).into()).await?;
        }
        self.integration_conversation_of(integration, id).await
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
        let id = &self.integration_conversation_of(integration, id).await?.id;
        let message_id = message.id.unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        let content = vec![ClientContent::Text { text: message.text }];
        let frame = if message.steer {
            ClientFrame::Steer {
                steer_id: BlockId::try_from(message_id.clone())
                    .map_err(|_| IntegrationRefusal::Refused("The message id is no id.".to_owned()))?,
                content,
                comments: Vec::new(),
            }
        } else {
            ClientFrame::Send(SendMessage {
                message_id: TurnId::try_from(message_id.clone())
                    .map_err(|_| IntegrationRefusal::Refused("The message id is no id.".to_owned()))?,
                content,
                comments: Vec::new(),
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
        Ok(())
    }

    /// Stops every agent of the integration's conversation `id`, as Stop does.
    pub async fn stop_integration_conversation(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        id: &ConversationId,
    ) -> Result<(), IntegrationRefusal> {
        let id = &self.integration_conversation_of(integration, id).await?.id;
        self.admit_frame(id, ClientFrame::AbortSubagents {}).await?;
        self.admit_frame(id, ClientFrame::Abort {}).await?;
        Ok(())
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
        let cloud = self
            .host_shard()
            .resolve_target(&record)
            .await
            .ok()
            .and_then(|target| target.device().cloned());
        let summary = self.conversation_summary(record).await?;
        Ok(api::Conversation {
            id: summary.id,
            key,
            cloud,
            title: summary.title.unwrap_or_default(),
            status: summary.status,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
        })
    }
}

/// An agent entry as an integration sees it; none for a side question,
/// which is the web app's.
fn api_agent(entry: AgentEntry) -> Option<api::Agent> {
    let running = !matches!(entry.phase, SessionPhase::Idle);
    match entry.role {
        AgentRole::Root => Some(api::Agent {
            id: entry.id,
            number: 0,
            parent: None,
            description: String::new(),
            running,
            cloud: None,
        }),
        AgentRole::Subagent(child) => Some(api::Agent {
            id: entry.id,
            number: child.number,
            parent: Some(child.parent),
            description: child.description,
            running,
            cloud: child.device.and_then(|device| DeviceId::try_from(device).ok()),
        }),
        AgentRole::Side(_) => None,
    }
}

/// The sockets of the user's integrations (`GET /v1/socket`), each of which
/// hears every change of its integration's conversations, and the one that
/// serves an integration's plugin (`crate::integration_plugins`).
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
    pub(crate) next_call: Cell<u64>,
}

/// One socket of an integration's.
struct Socket {
    id: u64,
    sender: mpsc::Sender<api::Event>,
    /// Whether it declared the integration's plugin, so its requests may go
    /// to it.
    serves: bool,
}

/// How many events a socket may hold that its program has not read: a
/// program that stops reading would otherwise hold the backend's memory
/// without end. A socket that falls this far behind is ended, and its
/// program reads again what it shows, as after any reconnection.
const SOCKET_BACKLOG: usize = 4096;

impl IntegrationEvents {
    /// A socket of `integration`'s from now on, by its number, until its
    /// receiver goes.
    pub fn subscribe(&self, integration: IntegrationId) -> (u64, mpsc::Receiver<api::Event>) {
        let (sender, receiver) = mpsc::channel(SOCKET_BACKLOG);
        let id = self.next_socket.get() + 1;
        self.next_socket.set(id);
        self.sockets.borrow_mut().entry(integration).or_default().push(Socket {
            id,
            sender,
            serves: false,
        });
        (id, receiver)
    }

    /// Tells every socket of `integration`'s; a socket whose receiver went,
    /// or that fell behind, is dropped.
    fn publish(&self, integration: &IntegrationId, event: api::Event) {
        let mut sockets = self.sockets.borrow_mut();
        let Some(list) = sockets.get_mut(integration) else {
            return;
        };
        list.retain(|socket| socket.sender.try_send(event.clone()).is_ok());
        if list.is_empty() {
            sockets.remove(integration);
        }
    }

    /// Tells the socket `socket` of `integration`'s alone; false when it is
    /// gone or fell behind, which drops it.
    pub(crate) fn send_to(&self, integration: &IntegrationId, socket: u64, event: api::Event) -> bool {
        let mut sockets = self.sockets.borrow_mut();
        let Some(list) = sockets.get_mut(integration) else {
            return false;
        };
        let Some(index) = list.iter().position(|candidate| candidate.id == socket) else {
            return false;
        };
        if list[index].sender.try_send(event).is_ok() {
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
    }

    /// A socket of `integration`'s that serves its plugin, the one that
    /// declared last.
    pub(crate) fn server(&self, integration: &IntegrationId) -> Option<u64> {
        self.sockets
            .borrow()
            .get(integration)?
            .iter()
            .rev()
            .find(|socket| socket.serves && !socket.sender.is_closed())
            .map(|socket| socket.id)
    }

    /// The socket `socket` of `integration`'s went: its requests fail.
    pub(crate) fn closed(&self, integration: &IntegrationId, socket: u64) {
        if let Some(list) = self.sockets.borrow_mut().get_mut(integration) {
            list.retain(|candidate| candidate.id != socket);
        }
        let ended: Vec<u64> = self
            .calls
            .borrow()
            .iter()
            .filter(|(_, call)| call.integration == *integration && call.socket == socket)
            .map(|(id, _)| *id)
            .collect();
        for id in ended {
            if let Some(call) = self.calls.borrow_mut().remove(&id) {
                let _ = call.reply.send(api::PluginReply::Failed {
                    message: "the integration's program went away".to_owned(),
                });
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
    /// receiver goes.
    pub fn subscribe_integration(&self, integration: IntegrationId) -> (u64, mpsc::Receiver<api::Event>) {
        self.integration_events.subscribe(integration)
    }

    /// The conversation's status may have changed: an integration's sockets
    /// hear its status now. A tree that starts working says `running` in the
    /// step it starts, before anything it does; what it ends with is read
    /// after.
    pub(crate) fn integration_status_changed(self: &Rc<Self>, conversation: &ConversationId) {
        if self.agent().tree(&root_of(conversation)).is_some_and(|tree| tree.works())
            && let Some(integration) = self.integration_of_open(conversation)
        {
            self.integration_events.publish(
                &integration,
                api::Event::Conversation {
                    conversation: conversation.clone(),
                    status: ConversationStatus::Running,
                },
            );
            return;
        }
        // A tree that closed has no observer any more.
        if self.agent().tree(&root_of(conversation)).is_none() {
            self.integration_events.observed.borrow_mut().remove(conversation);
        }
        let shard = self.clone();
        let conversation = conversation.clone();
        self.tasks().spawn_local(async move {
            let control = &shard.services().control;
            let Ok(Some(kept)) = control.integration_conversation(conversation.clone()).await else {
                return;
            };
            if !shard.integration_events.heard(&kept.integration) {
                return;
            }
            let Ok(Some(record)) = control.conversation(conversation.clone()).await else {
                return;
            };
            if let Ok(summary) = shard.conversation_summary(record).await {
                // A turn that started meanwhile told its own status in its
                // step; this one's would come after it, and be older.
                let works = shard.agent().tree(&root_of(&conversation)).is_some_and(|tree| tree.works());
                if works != matches!(summary.status, ConversationStatus::Running | ConversationStatus::Compacting) {
                    return;
                }
                shard.integration_events.publish(
                    &kept.integration,
                    api::Event::Conversation {
                        conversation,
                        status: summary.status,
                    },
                );
            }
        });
    }
}
