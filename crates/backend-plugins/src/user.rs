//! One user's plugins on the user's shard (`plugins.md` § The plugin
//! host, § A user's plugins): an instance of every plugin, made when the
//! shard starts; which of them the user has on, which decides the toolset a
//! conversation's tree opens with and which page states, page calls and
//! user streams exist; the context sources among them; the Host directories
//! of the plugins on; and the answers to their port operations. The plugin
//! host answers values, directories and changes itself; what reaches the
//! user's blobs, a conversation's Hosts or the user's exposes goes to the
//! product, which owns the blob namespace and the conversation's host
//! access.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use demi_backend_database::StorageError;
use demi_backend_database::blob_refs::OwnerBlobs;
use demi_backend_database::control::ControlService;
use demi_backend_database::panels::PanelOutcome;
use demi_backend_database::plugin_values::{ValueWrite, Written};
use demi_backend_page_sync::{Part, UserMarks};
use demi_command_declarations::NativeOperation;
use demi_host_interface::{CommandSet, GroupBuilder, PortError, RpcPort};
use demi_plugin_interface::{
    CallKind, ConversationHost, DirectoryPath, ExposeList, ExposeRecord, HostDirectory, HostFile,
    HostRead, PanelTabChange, Plugin, PluginError, PluginId, PluginPort, PluginTransport,
    PortAnswer, PortFailure, PortMessage, PortRefusal, Reply, Request, Scope, StoredValue, Topic,
};
use demi_shared_types::{B64Bytes, BlobRef, NodeId, TurnId};
use demi_web_api_protocol::conversations::PluginRevision;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, ExposeId, UserId};
use demi_web_api_protocol::panel::{PanelChange, PanelEffect, WorkPanel};
use demi_web_api_protocol::plugins::{PluginEntry, PluginStateAnswer};
use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::Registry;
use crate::commands::compose;

/// What the product does for the plugins' port: the operations that reach
/// a conversation's Hosts through its host access, and the user's exposes.
/// The user's shard implements it.
pub trait ProductPort {
    /// Runs `operation` once on the conversation's primary Host for the user,
    /// waking it or not as `kind` says, and answers its JSON result.
    fn package_call<'a>(
        &'a self,
        conversation: &'a ConversationId,
        operation: &'a NativeOperation,
        args: Map<String, Value>,
        kind: CallKind,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<Value, PortFailure>>;

    fn conversation_hosts<'a>(
        &'a self,
        conversation: &'a ConversationId,
    ) -> LocalBoxFuture<'a, Result<Vec<ConversationHost>, PortFailure>>;

    /// Reads `reads` on the conversation's primary Host in the form that never
    /// wakes it; [`PortRefusal::NotRunning`] when it is not running.
    fn read_host_files<'a>(
        &'a self,
        conversation: &'a ConversationId,
        reads: Vec<HostRead>,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<Vec<HostFile>, PortFailure>>;

    /// Stores `bytes` in the user's blob namespace.
    fn put_blob(&self, bytes: B64Bytes) -> LocalBoxFuture<'_, Result<BlobRef, PortFailure>>;

    fn get_blob(&self, blob: BlobRef) -> LocalBoxFuture<'_, Result<Option<B64Bytes>, PortFailure>>;

    /// The record of the user's blob uses, which each write of a plugin
    /// value or of a plugin's Host directories updates before it commits.
    fn blob_uses(&self) -> Arc<dyn OwnerBlobs>;

    fn exposes(&self) -> LocalBoxFuture<'_, Result<ExposeList, PortFailure>>;

    fn create_expose(
        &self,
        device: DeviceId,
        address: String,
        lifetime: u64,
    ) -> LocalBoxFuture<'_, Result<ExposeRecord, PortFailure>>;

    fn renew_expose(
        &self,
        expose: ExposeId,
        lifetime: u64,
    ) -> LocalBoxFuture<'_, Result<ExposeRecord, PortFailure>>;

    fn remove_expose(&self, expose: ExposeId) -> LocalBoxFuture<'_, Result<(), PortFailure>>;
}

/// A page's call of a plugin method.
#[derive(Debug, Clone)]
pub struct PageCall {
    pub plugin: String,
    pub method: String,
    /// The JSON body, which the method's schema checks.
    pub params: Value,
    /// The conversation of the route, for a method of the conversation
    /// scope.
    pub conversation: Option<ConversationId>,
}

/// Why a page call did not answer.
#[derive(Debug, thiserror::Error)]
pub enum PageCallError {
    #[error("No plugin \"{0}\"")]
    UnknownPlugin(String),
    #[error("The plugin \"{0}\" is off")]
    Disabled(String),
    #[error("The plugin \"{plugin}\" has no method \"{method}\" here")]
    UnknownMethod { plugin: String, method: String },
    #[error("{0}")]
    InvalidParams(String),
    #[error(transparent)]
    Plugin(#[from] PluginError),
}

/// Why a change of a work panel was refused (`web-api.md` § Work panel
/// state).
#[derive(Debug, thiserror::Error)]
pub enum PanelError {
    #[error("No plugin the user has on declares the panel kind \"{0}\"")]
    UnknownKind(String),
    #[error("The work panel holds as many tabs, or as much data, as it can")]
    Full,
    #[error("The work panel's tabs would be over their size")]
    TooLarge,
    #[error("The conversation is archived")]
    Archived,
    #[error("No conversation of that id")]
    Missing,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl PanelError {
    /// The refusal as the port gives it to a plugin.
    fn refusal(self) -> PortFailure {
        let code = match &self {
            Self::UnknownKind(_) => ErrorCode::UnknownPanelKind,
            Self::Full => ErrorCode::PanelFull,
            Self::TooLarge => ErrorCode::TooLarge,
            Self::Archived => ErrorCode::ConversationArchived,
            Self::Missing => ErrorCode::ConversationNotFound,
            Self::Storage(_) => {
                let Self::Storage(error) = self else {
                    unreachable!("matched as a storage failure")
                };
                return storage(error);
            }
        };
        PortFailure::Refused(PortRefusal::Panel {
            code,
            message: self.to_string(),
        })
    }
}

/// Why a plugin could not be turned on or off.
#[derive(Debug, thiserror::Error)]
pub enum SwitchError {
    #[error("No plugin \"{0}\"")]
    UnknownPlugin(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// The commands and revision of the plugins a user has on, which a
/// conversation's tree opens with.
pub struct PluginToolset {
    pub commands: CommandSet,
    /// The ids of the plugins on that declare commands, in registration
    /// order: two sets of the same commands have the same revision.
    pub revision: String,
}

/// What every request of the user's plugins shares.
pub(crate) struct Shared {
    registry: Arc<Registry>,
    pub(crate) user: UserId,
    instances: Vec<Rc<dyn Plugin>>,
    product: Rc<dyn ProductPort>,
    control: ControlService,
    marks: UserMarks,
    /// Whether the user has each plugin on, by its index, once read: every
    /// switch goes through this shard, so the copy stays current.
    enabled: RefCell<Option<Vec<bool>>>,
    /// Cancelled when the user turns the plugin of its index off, which
    /// ends the plugin's open user streams.
    stream_ends: RefCell<Vec<CancellationToken>>,
    /// Each plugin's Host directories for the user, by its index, once
    /// read: every change goes through this shard.
    directories: RefCell<Option<Vec<Vec<HostDirectory>>>>,
    /// How many times each plugin's conversation state changed since the
    /// shard started, by the plugin's index and the conversation; a pair
    /// without an entry has changed none (`web-api.md` § Conversation state
    /// of plugins).
    revisions: RefCell<HashMap<(usize, ConversationId), u64>>,
}

impl Shared {
    /// Sends `request` to plugin `plugin`, with a port for the request's
    /// conversation and, for a command, the call's rpc port.
    pub(crate) async fn request(
        self: &Rc<Self>,
        plugin: usize,
        request: Request,
        conversation: Option<ConversationId>,
        rpc: Option<RpcPort>,
        cancel: CancellationToken,
    ) -> Result<Reply, PluginError> {
        let transport = RequestPort {
            shared: self.clone(),
            plugin,
            conversation,
            rpc,
            cancel: cancel.clone(),
        };
        let port = PluginPort::new(Rc::new(transport), cancel);
        self.instances[plugin].call(request, port).await
    }

    fn plugin_id(&self, plugin: usize) -> String {
        self.registry.plugins[plugin].id().as_str().to_owned()
    }

    fn revision(&self, plugin: usize, conversation: &ConversationId) -> u64 {
        let key = (plugin, conversation.clone());
        self.revisions.borrow().get(&key).copied().unwrap_or(0)
    }

    /// Marks plugin `plugin`'s page state of `scope` changed: the user's
    /// goes to every page of the user; a conversation's raises its revision,
    /// which the conversation's summary carries.
    fn changed(&self, plugin: usize, conversation: Option<&ConversationId>) {
        match conversation {
            None => self.marks.mark(Part::Plugin(self.plugin_id(plugin))),
            Some(conversation) => {
                let key = (plugin, conversation.clone());
                *self.revisions.borrow_mut().entry(key).or_default() += 1;
                self.marks.mark(Part::Conversation(conversation.clone()));
            }
        }
    }

    /// Applies `change` to the conversation's panel and marks its summary,
    /// whose revision the pages read the panel by; answers the panel's
    /// revision and what the change did.
    async fn apply_panel(
        &self,
        conversation: &ConversationId,
        changes: Vec<PanelChange>,
    ) -> Result<(u64, Vec<PanelEffect>), PanelError> {
        match self
            .control
            .change_panel(conversation.clone(), changes)
            .await?
        {
            PanelOutcome::Changed { revision, effects } => {
                self.marks.mark(Part::Conversation(conversation.clone()));
                Ok((revision, effects))
            }
            PanelOutcome::Unchanged { revision } => Ok((revision, Vec::new())),
            PanelOutcome::Full => Err(PanelError::Full),
            PanelOutcome::TooLarge => Err(PanelError::TooLarge),
            PanelOutcome::Archived => Err(PanelError::Archived),
            PanelOutcome::Missing => Err(PanelError::Missing),
        }
    }

    /// Tells the plugin that owns the kind of the tab the user created or
    /// removed, if the user has it on, after the change is answered: the
    /// page never waits for the plugin's work, which may wake a Host.
    fn tell_owner(
        self: &Rc<Self>,
        conversation: ConversationId,
        effect: PanelEffect,
        enabled: &[bool],
    ) {
        let (change, tab) = match effect {
            PanelEffect::Created(tab) => (PanelTabChange::Created, tab),
            PanelEffect::Removed(tab) => (PanelTabChange::Removed, tab),
            PanelEffect::Updated | PanelEffect::Moved => return,
        };
        let Some(owner) = self
            .registry
            .kind_owner(&tab.kind)
            .filter(|owner| enabled[*owner])
        else {
            return;
        };
        let request = Request::PanelTab {
            user: self.user.clone(),
            conversation: conversation.clone(),
            change,
            tab,
        };
        self.spawn_request(owner, request, conversation);
    }

    /// Sends `request` about `conversation` to plugin `plugin` without
    /// waiting for it; what it could not do is logged, since nobody waits
    /// for its answer.
    fn spawn_request(
        self: &Rc<Self>,
        plugin: usize,
        request: Request,
        conversation: ConversationId,
    ) {
        let shared = self.clone();
        tokio::task::spawn_local(async move {
            let asked = shared
                .request(
                    plugin,
                    request,
                    Some(conversation),
                    None,
                    CancellationToken::new(),
                )
                .await;
            if let Err(error) = asked {
                tracing::warn!(
                    plugin = shared.plugin_id(plugin),
                    %error,
                    "a plugin could not do its part of a change"
                );
            }
        });
    }

    /// Each plugin's Host directories, by its index.
    async fn directories(&self) -> Result<Vec<Vec<HostDirectory>>, StorageError> {
        if let Some(directories) = &*self.directories.borrow() {
            return Ok(directories.clone());
        }
        let mut stored = self.control.plugin_directories(self.user.clone()).await?;
        let directories: Vec<Vec<HostDirectory>> = self
            .registry
            .plugins
            .iter()
            .map(|registered| stored.remove(registered.id().as_str()).unwrap_or_default())
            .collect();
        self.directories.replace(Some(directories.clone()));
        Ok(directories)
    }

    /// Replaces plugin `plugin`'s Host directories and answers each one's
    /// path on every Host.
    async fn set_directories(
        &self,
        plugin: usize,
        directories: Vec<HostDirectory>,
    ) -> Result<Vec<DirectoryPath>, PortFailure> {
        HostDirectory::check_set(&directories).map_err(PortError::Failed)?;
        let mut current = self.directories().await.map_err(storage)?;
        self.control
            .set_plugin_directories(
                self.user.clone(),
                self.plugin_id(plugin),
                directories.clone(),
                self.product.blob_uses(),
            )
            .await
            .map_err(storage)?
            .map_err(|refused| PortError::Failed(refused.to_string()))?;
        let id = self.registry.plugins[plugin].id();
        let paths = directories
            .iter()
            .map(|directory| DirectoryPath {
                name: directory.name.clone(),
                path: directory.path(id),
            })
            .collect();
        current[plugin] = directories;
        self.directories.replace(Some(current));
        Ok(paths)
    }
}

/// One user's plugins. Cloning it is cheap.
#[derive(Clone)]
pub struct UserPlugins(Rc<Shared>);

impl UserPlugins {
    /// An instance of every plugin of `registry` for `user`, whose port
    /// reaches `product`, the user's values in `control` and the user's
    /// pages through `marks`.
    pub fn new(
        registry: Arc<Registry>,
        user: UserId,
        product: Rc<dyn ProductPort>,
        control: ControlService,
        marks: UserMarks,
    ) -> Self {
        let instances = registry
            .plugins
            .iter()
            .map(|registered| registered.factory.instance())
            .collect();
        let stream_ends = registry
            .plugins
            .iter()
            .map(|_| CancellationToken::new())
            .collect();
        Self(Rc::new(Shared {
            registry,
            user,
            instances,
            product,
            control,
            marks,
            enabled: RefCell::new(None),
            stream_ends: RefCell::new(stream_ends),
            directories: RefCell::new(None),
            revisions: RefCell::new(HashMap::new()),
        }))
    }

    /// Whether the user has each plugin on, by its index.
    async fn enabled(&self) -> Result<Vec<bool>, StorageError> {
        if let Some(enabled) = &*self.0.enabled.borrow() {
            return Ok(enabled.clone());
        }
        let choices = self.0.control.user_plugins(self.0.user.clone()).await?;
        let enabled: Vec<bool> = self
            .0
            .registry
            .plugins
            .iter()
            .map(|registered| {
                choices
                    .get(registered.id().as_str())
                    .copied()
                    .unwrap_or(true)
            })
            .collect();
        self.0.enabled.replace(Some(enabled.clone()));
        Ok(enabled)
    }

    /// The backend's plugins with whether the user has each on, for the
    /// settings page.
    pub async fn entries(&self) -> Result<Vec<PluginEntry>, StorageError> {
        let enabled = self.enabled().await?;
        Ok(self
            .0
            .registry
            .plugins
            .iter()
            .zip(enabled)
            .map(|(registered, enabled)| {
                let manifest = registered.factory.manifest();
                PluginEntry {
                    id: manifest.id.to_string(),
                    name: manifest.name.clone(),
                    description: manifest.description.clone(),
                    enabled,
                    packages: registered.packages.iter().cloned().collect(),
                }
            })
            .collect())
    }

    /// Turns `plugin` on or off for the user, and marks the plugin list and
    /// the plugin's state as changed. A plugin turned off ends its open user
    /// streams. Answers whether the choice changed.
    pub async fn switch(&self, plugin: &str, enabled: bool) -> Result<bool, SwitchError> {
        let Some((index, registered)) = self.0.registry.plugin(plugin) else {
            return Err(SwitchError::UnknownPlugin(plugin.to_owned()));
        };
        let mut current = self.enabled().await?;
        if current[index] == enabled {
            return Ok(false);
        }
        self.0
            .control
            .set_user_plugin(self.0.user.clone(), plugin.to_owned(), enabled)
            .await?;
        current[index] = enabled;
        self.0.enabled.replace(Some(current));
        if !enabled {
            let ended = std::mem::take(&mut self.0.stream_ends.borrow_mut()[index]);
            ended.cancel();
        }
        self.0.marks.mark(Part::Plugins);
        self.0
            .marks
            .mark(Part::Plugin(registered.id().as_str().to_owned()));
        Ok(true)
    }

    /// The toolset of the plugins the user has on: their commands, with the
    /// product's `product` groups under `demi`.
    pub async fn toolset(&self, product: Vec<GroupBuilder>) -> Result<PluginToolset, StorageError> {
        let enabled = self.enabled().await?;
        let commands = compose(&self.0.registry, Some(&self.0), product, |index| {
            enabled[index]
        })
        .expect("the plugins' commands were checked at startup");
        Ok(PluginToolset {
            commands,
            revision: self.revision_of(&enabled),
        })
    }

    /// The revision of the toolset the user's plugins give now.
    pub async fn revision(&self) -> Result<String, StorageError> {
        let enabled = self.enabled().await?;
        Ok(self.revision_of(&enabled))
    }

    fn revision_of(&self, enabled: &[bool]) -> String {
        let ids: Vec<&str> = self
            .0
            .registry
            .plugins
            .iter()
            .zip(enabled)
            .filter(|(registered, enabled)| **enabled && !registered.commands.is_empty())
            .map(|(registered, _)| registered.id().as_str())
            .collect();
        ids.join(",")
    }

    /// Asks the context source `plugin` for a node's new context block;
    /// nothing while the user has it off.
    pub async fn context(
        &self,
        plugin: &PluginId,
        asked: ContextAsk,
        cancel: CancellationToken,
    ) -> Result<Option<String>, PluginError> {
        let Some((index, _)) = self.0.registry.plugin(plugin.as_str()) else {
            return Ok(None);
        };
        let enabled = self.enabled().await.map_err(PluginError::failed)?;
        if !enabled[index] {
            return Ok(None);
        }
        let request = Request::Context {
            user: self.0.user.clone(),
            conversation: asked.conversation.clone(),
            node: asked.node,
            cwd: asked.cwd,
            turn: asked.turn,
            seen: asked.seen,
        };
        let reply = self
            .0
            .request(index, request, Some(asked.conversation), None, cancel)
            .await?;
        match reply {
            Reply::Context { text } => Ok(text),
            reply => Err(PluginError::failed(format!(
                "the plugin answered a context request with {reply:?}"
            ))),
        }
    }

    /// Every plugin's Host directories for the user, which host access
    /// installs before a job: a plugin the user has off has none, so a Host
    /// loses its directories at its next installation.
    pub async fn directories(&self) -> Result<Vec<(PluginId, Vec<HostDirectory>)>, StorageError> {
        let enabled = self.enabled().await?;
        let directories = self.0.directories().await?;
        Ok(self
            .0
            .registry
            .plugins
            .iter()
            .zip(directories)
            .zip(enabled)
            .map(|((registered, directories), enabled)| {
                let set = if enabled { directories } else { Vec::new() };
                (registered.id().clone(), set)
            })
            .collect())
    }

    /// What ends the user stream `name` once its plugin is turned off; none
    /// while the user has its plugin off, when the stream does not exist.
    pub async fn stream_end(&self, name: &str) -> Result<Option<CancellationToken>, StorageError> {
        let Some(index) = self.0.registry.stream_owner(name) else {
            return Ok(None);
        };
        if !self.enabled().await?[index] {
            return Ok(None);
        }
        Ok(Some(self.0.stream_ends.borrow()[index].clone()))
    }

    /// The user state of each plugin the user has on that declares one, by
    /// id, for the product state.
    pub async fn page_states(&self) -> Result<BTreeMap<String, Value>, PluginError> {
        let enabled = self.enabled().await.map_err(PluginError::failed)?;
        let mut states = BTreeMap::new();
        for (index, registered) in self.0.registry.plugins.iter().enumerate() {
            if enabled[index] && registered.state(Scope::User).is_some() {
                let state = self.state_of(index, None, CancellationToken::new()).await?;
                states.insert(registered.id().as_str().to_owned(), state);
            }
        }
        Ok(states)
    }

    /// The user state of `plugin`; none for a plugin the user has off or
    /// whose page declares none.
    pub async fn page_state(&self, plugin: &str) -> Result<Option<Value>, PluginError> {
        let Some((index, registered)) = self.0.registry.plugin(plugin) else {
            return Ok(None);
        };
        let enabled = self.enabled().await.map_err(PluginError::failed)?;
        if !enabled[index] || registered.state(Scope::User).is_none() {
            return Ok(None);
        }
        self.state_of(index, None, CancellationToken::new())
            .await
            .map(Some)
    }

    /// The conversation state of `plugin` for `conversation`, with the
    /// revision it was read at: read after the revision, so a change made
    /// while it is read raises the revision past it.
    pub async fn conversation_state(
        &self,
        plugin: &str,
        conversation: ConversationId,
        cancel: CancellationToken,
    ) -> Result<PluginStateAnswer, PageCallError> {
        let Some((index, registered)) = self.0.registry.plugin(plugin) else {
            return Err(PageCallError::UnknownPlugin(plugin.to_owned()));
        };
        if registered.state(Scope::Conversation).is_none() {
            return Err(PageCallError::UnknownPlugin(plugin.to_owned()));
        }
        let enabled = self.enabled().await.map_err(PluginError::failed)?;
        if !enabled[index] {
            return Err(PageCallError::Disabled(plugin.to_owned()));
        }
        let revision = self.0.revision(index, &conversation);
        let state = self.state_of(index, Some(conversation), cancel).await?;
        Ok(PluginStateAnswer { revision, state })
    }

    /// The revision of each plugin's conversation state for `conversation`,
    /// for the conversation's summary: every plugin that declares one, so
    /// turning a plugin on changes no summary.
    pub fn plugin_revisions(&self, conversation: &ConversationId) -> Vec<PluginRevision> {
        self.0
            .registry
            .plugins
            .iter()
            .enumerate()
            .filter(|(_, registered)| registered.state(Scope::Conversation).is_some())
            .map(|(index, registered)| PluginRevision {
                plugin: registered.id().as_str().to_owned(),
                revision: self.0.revision(index, conversation),
            })
            .collect()
    }

    /// `topic` fired, for the user or for `conversation`: the state of
    /// each plugin that follows it is marked changed, and each plugin told
    /// about it that the user has on is told (`plugins.md` § Topics).
    pub fn fire(&self, topic: Topic, conversation: Option<&ConversationId>) {
        let followers: Vec<usize> = self
            .0
            .registry
            .following(topic)
            .map(|(index, _)| index)
            .collect();
        for plugin in followers {
            self.0.changed(plugin, conversation);
        }
        let told: Vec<usize> = self.0.registry.told(topic).collect();
        // Every topic a plugin is told about is a conversation's.
        let Some(conversation) = conversation.filter(|_| !told.is_empty()).cloned() else {
            return;
        };
        let plugins = self.clone();
        tokio::task::spawn_local(async move {
            let enabled = match plugins.enabled().await {
                Ok(enabled) => enabled,
                Err(error) => {
                    tracing::warn!(%error, "the plugins told about a topic could not be read");
                    return;
                }
            };
            for plugin in told.into_iter().filter(|plugin| enabled[*plugin]) {
                let request = Request::Topic {
                    user: plugins.0.user.clone(),
                    topic,
                    conversation: Some(conversation.clone()),
                };
                plugins
                    .0
                    .spawn_request(plugin, request, conversation.clone());
            }
        });
    }

    /// The conversation's work panel.
    pub async fn panel(&self, conversation: ConversationId) -> Result<WorkPanel, StorageError> {
        self.0.control.panel(conversation).await
    }

    /// A page's changes of the conversation's work panel, all or none: each
    /// tab they create is of a kind a plugin the user has on declares.
    /// Answers the panel's revision once the changes are in it, and tells
    /// the kind's plugin of each tab the user created or removed.
    pub async fn change_panel(
        &self,
        conversation: ConversationId,
        changes: Vec<PanelChange>,
    ) -> Result<u64, PanelError> {
        let enabled = self.enabled().await?;
        for change in &changes {
            if let PanelChange::Create(create) = change {
                let owned = self
                    .0
                    .registry
                    .kind_owner(&create.kind)
                    .is_some_and(|owner| enabled[owner]);
                if !owned {
                    return Err(PanelError::UnknownKind(create.kind.clone()));
                }
            }
        }
        let (revision, effects) = self.0.apply_panel(&conversation, changes).await?;
        for effect in effects {
            self.0.tell_owner(conversation.clone(), effect, &enabled);
        }
        Ok(revision)
    }

    async fn state_of(
        &self,
        plugin: usize,
        conversation: Option<ConversationId>,
        cancel: CancellationToken,
    ) -> Result<Value, PluginError> {
        let request = Request::PageState {
            user: self.0.user.clone(),
            conversation: conversation.clone(),
        };
        let reply = self
            .0
            .request(plugin, request, conversation, None, cancel)
            .await?;
        match reply {
            Reply::State { state } => Ok(state),
            reply => Err(PluginError::failed(format!(
                "the plugin answered its page state with {reply:?}"
            ))),
        }
    }

    /// Calls a page method: the plugin has it for the call's scope, and
    /// its parameters fit its schema.
    pub async fn page_call(
        &self,
        call: PageCall,
        cancel: CancellationToken,
    ) -> Result<Value, PageCallError> {
        let Some((index, registered)) = self.0.registry.plugin(&call.plugin) else {
            return Err(PageCallError::UnknownPlugin(call.plugin));
        };
        let enabled = self.enabled().await.map_err(PluginError::failed)?;
        if !enabled[index] {
            return Err(PageCallError::Disabled(call.plugin));
        }
        let scope = match call.conversation {
            Some(_) => Scope::Conversation,
            None => Scope::User,
        };
        let method = registered.page.as_ref().and_then(|page| {
            page.methods
                .iter()
                .find(|method| method.name == call.method && method.scope == scope)
        });
        let Some(method) = method else {
            return Err(PageCallError::UnknownMethod {
                plugin: call.plugin,
                method: call.method,
            });
        };
        method
            .params
            .check(&call.params)
            .map_err(PageCallError::InvalidParams)?;
        let Value::Object(params) = call.params else {
            return Err(PageCallError::InvalidParams(
                "the parameters are not an object".into(),
            ));
        };
        let request = Request::PageCall {
            user: self.0.user.clone(),
            method: call.method,
            params,
            conversation: call.conversation.clone(),
        };
        let reply = self
            .0
            .request(index, request, call.conversation, None, cancel)
            .await?;
        match reply {
            Reply::Result { result } => Ok(result),
            reply => Err(PluginError::failed(format!(
                "the plugin answered a page call with {reply:?}"
            ))
            .into()),
        }
    }
}

/// What a node gives a context source before a provider request.
pub struct ContextAsk {
    pub conversation: ConversationId,
    pub node: NodeId,
    /// The node's working directory.
    pub cwd: String,
    /// The node's current input turn.
    pub turn: TurnId,
    /// The text of the source's own blocks the model receives, oldest
    /// first.
    pub seen: Vec<String>,
}

/// One request's port: the call's rpc port for a command, the user's
/// values and pages, and the product's operations for the request's
/// conversation.
struct RequestPort {
    shared: Rc<Shared>,
    plugin: usize,
    conversation: Option<ConversationId>,
    rpc: Option<RpcPort>,
    cancel: CancellationToken,
}

impl RequestPort {
    async fn answer(&self, message: PortMessage) -> Result<PortAnswer, PortFailure> {
        let shared = &self.shared;
        let product = &*shared.product;
        let answer = match message {
            PortMessage::Rpc { request } => {
                let Some(rpc) = &self.rpc else {
                    return Err(PortError::Unexpected {
                        asked: "rpc",
                        answered: "no rpc port: the request is not a command",
                    }
                    .into());
                };
                PortAnswer::Rpc {
                    response: rpc.forward(request).await?,
                }
            }
            PortMessage::ReadValue { key } => {
                let value = shared
                    .control
                    .plugin_value(shared.user.clone(), shared.plugin_id(self.plugin), key)
                    .await
                    .map_err(storage)?;
                PortAnswer::Value {
                    value: value.map(stored),
                }
            }
            PortMessage::ListValues => {
                let values = shared
                    .control
                    .plugin_values(shared.user.clone(), shared.plugin_id(self.plugin))
                    .await
                    .map_err(storage)?;
                PortAnswer::Values {
                    values: values
                        .into_iter()
                        .map(|(key, value)| (key, stored(value)))
                        .collect(),
                }
            }
            PortMessage::WriteValue {
                key,
                value,
                revision,
                blobs,
            } => {
                let write = ValueWrite {
                    user: shared.user.clone(),
                    plugin: shared.plugin_id(self.plugin),
                    key,
                    document: value,
                    revision,
                    blobs,
                };
                let written = shared
                    .control
                    .write_plugin_value(write, product.blob_uses())
                    .await
                    .map_err(storage)?;
                match written {
                    Written::Revision(revision) => PortAnswer::Written { revision },
                    Written::Conflict => return Err(PortFailure::Refused(PortRefusal::Conflict)),
                    Written::Refused(refused) => {
                        return Err(PortError::Failed(refused.to_string()).into());
                    }
                }
            }
            PortMessage::RemoveValue { key, revision } => {
                let removed = shared
                    .control
                    .remove_plugin_value(
                        shared.user.clone(),
                        shared.plugin_id(self.plugin),
                        key,
                        revision,
                        product.blob_uses(),
                    )
                    .await
                    .map_err(storage)?;
                match removed {
                    Written::Revision(_) => PortAnswer::Done,
                    Written::Conflict => return Err(PortFailure::Refused(PortRefusal::Conflict)),
                    Written::Refused(refused) => {
                        return Err(PortError::Failed(refused.to_string()).into());
                    }
                }
            }
            PortMessage::PutBlob { bytes } => PortAnswer::Blob {
                blob: product.put_blob(bytes).await?,
            },
            PortMessage::GetBlob { blob } => PortAnswer::Bytes {
                bytes: product.get_blob(blob).await?,
            },
            PortMessage::SetDirectories { directories } => PortAnswer::Directories {
                paths: shared.set_directories(self.plugin, directories).await?,
            },
            PortMessage::ReadHostFiles { reads } => PortAnswer::HostFiles {
                files: product
                    .read_host_files(self.conversation()?, reads, &self.cancel)
                    .await?,
            },
            PortMessage::Changed { scope } => {
                let conversation = match scope {
                    Scope::User => None,
                    Scope::Conversation => Some(self.conversation()?),
                };
                shared.changed(self.plugin, conversation);
                PortAnswer::Done
            }
            PortMessage::PackageCall {
                operation,
                args,
                kind,
            } => {
                let registered = &shared.registry.plugins[self.plugin];
                if !registered.packages.contains(&operation.package) {
                    return Err(PortError::Unexpected {
                        asked: "package_call",
                        answered: "a package the plugin's commands do not bind",
                    }
                    .into());
                }
                let conversation = self.conversation()?;
                let result = product
                    .package_call(conversation, &operation, args, kind, &self.cancel)
                    .await?;
                PortAnswer::Called { result }
            }
            PortMessage::ConversationHosts => PortAnswer::Hosts {
                hosts: product.conversation_hosts(self.conversation()?).await?,
            },
            PortMessage::ListExposes => PortAnswer::Exposes {
                list: product.exposes().await?,
            },
            PortMessage::CreateExpose {
                device,
                address,
                lifetime,
            } => PortAnswer::Expose {
                expose: product.create_expose(device, address, lifetime).await?,
            },
            PortMessage::RenewExpose { expose, lifetime } => PortAnswer::Expose {
                expose: product.renew_expose(expose, lifetime).await?,
            },
            PortMessage::RemoveExpose { expose } => {
                product.remove_expose(expose).await?;
                PortAnswer::Done
            }
            PortMessage::PanelTabs => {
                let registered = &shared.registry.plugins[self.plugin];
                let mut panel = shared
                    .control
                    .panel(self.conversation()?.clone())
                    .await
                    .map_err(storage)?;
                panel.tabs.retain(|tab| registered.owns_kind(&tab.kind));
                PortAnswer::Panel { panel }
            }
            PortMessage::CreatePanelTab { tab } => {
                if !shared.registry.plugins[self.plugin].owns_kind(&tab.kind) {
                    return Err(PanelError::UnknownKind(tab.kind).refusal());
                }
                self.panel_change(PanelChange::Create(tab)).await?
            }
            PortMessage::UpdatePanelTab { id, data } => {
                self.own_tab(&id).await?;
                self.panel_change(PanelChange::Update { id, data }).await?
            }
            PortMessage::RemovePanelTab { id } => {
                self.own_tab(&id).await?;
                self.panel_change(PanelChange::Remove { id }).await?
            }
        };
        Ok(answer)
    }

    /// Applies a plugin's change of the request's conversation's panel.
    async fn panel_change(&self, change: PanelChange) -> Result<PortAnswer, PortFailure> {
        let conversation = self.conversation()?;
        let (revision, _) = self
            .shared
            .apply_panel(conversation, vec![change])
            .await
            .map_err(|error| error.refusal())?;
        Ok(PortAnswer::PanelRevision { revision })
    }

    /// Refuses a change of a tab of another plugin's kind; a tab the panel
    /// no longer has is the change's to answer.
    async fn own_tab(&self, id: &str) -> Result<(), PortFailure> {
        let panel = self
            .shared
            .control
            .panel(self.conversation()?.clone())
            .await
            .map_err(storage)?;
        let registered = &self.shared.registry.plugins[self.plugin];
        match panel.tabs.iter().find(|tab| tab.id == id) {
            Some(tab) if !registered.owns_kind(&tab.kind) => {
                Err(PanelError::UnknownKind(tab.kind.clone()).refusal())
            }
            _ => Ok(()),
        }
    }

    fn conversation(&self) -> Result<&ConversationId, PortFailure> {
        self.conversation
            .as_ref()
            .ok_or(PortFailure::Refused(PortRefusal::NoConversation))
    }
}

impl PluginTransport for RequestPort {
    fn request(&self, message: PortMessage) -> LocalBoxFuture<'_, Result<PortAnswer, PortError>> {
        Box::pin(async move {
            match self.answer(message).await {
                Ok(answer) => Ok(answer),
                Err(PortFailure::Refused(refusal)) => Ok(PortAnswer::Refused { refusal }),
                Err(PortFailure::Port(error)) => Err(error),
            }
        })
    }
}

fn stored(value: demi_backend_database::plugin_values::PluginValue) -> StoredValue {
    StoredValue {
        value: value.document,
        revision: value.revision,
    }
}

/// A storage failure as the port reports it: the operation failed, which a
/// plugin cannot cause.
fn storage(error: StorageError) -> PortFailure {
    PortFailure::Port(PortError::Failed(error.to_string()))
}
