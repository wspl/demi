//! One user's plugins on the user's shard.
//!
//! The module holds:
//!
//! - Which plugins the user has on, which decides the toolset of the user's
//!   conversations and which page states, page calls and user streams exist.
//! - An instance of each plugin on, made at its first request after the shard
//!   started or the plugin was turned on, and dropped, with every task it
//!   started, when it is turned off.
//! - The context sources among them.
//! - The Host directories of the plugins on.
//! - The answers to their port operations.
//!
//! The plugin host answers values, directories, changes, secrets, outbound
//! requests and sign-ins itself. What reaches the user's blobs, a
//! conversation's Hosts or the user's provider entries goes to the product,
//! which owns the blob namespace, the conversation's host access and the
//! providers. The product also owns the sealing of a secret, the client an
//! outbound request is sent with, the public URL and the time.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::panels::PanelOutcome;
use demi_backend_database::plugin_values::{ValueWrite, Written};
use demi_backend_page_sync::{Part, UserMarks};
use demi_command_declarations::NativeOperation;
use demi_host_interface::{CommandSet, GroupBuilder, JobCaller, PortError, RpcPort};
use demi_plugin_interface::{
    AskedVideo, AudioAsk, AudioVoices, ImageAsk, MadeAudio, MadeImage, MediaListing, MediaRefusal, VideoAsk, VideoStep,
    CallKind, DirectoryPath, HostDirectory, HostFile, HostRead, Plugin,
    PluginError, PluginId, PluginPort, PluginTransport, PortAnswer, PortFailure, PortMessage,
    PortRefusal, Reply, Request, Scope, StoredValue,
};
use demi_shared_types::{B64Bytes, BlobRef, MediaKind, NodeId, Timestamp, TurnId};
use demi_web_api_protocol::conversations::PluginRevision;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use demi_web_api_protocol::panel::{HeldState, PanelChange, PanelEffect, PanelRead, WorkPanel};
use demi_web_api_protocol::plugins::{PluginEntry, PluginStateAnswer};
use demi_web_api_protocol::workflows::WatchSubject;
use futures_util::StreamExt as _;
use futures_util::future::LocalBoxFuture;
use futures_util::stream::LocalBoxStream;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::Registry;
use crate::commands::compose;
use crate::oauth::{Callback, CallbackError, SignIns};
use crate::web;

/// A video's job as the product runs it: each change of its state, then
/// its video or the failure that ended it. Dropped before its video, the
/// job's waiting ends and the job is cancelled where the vendor can cancel
/// one.
pub type VideoJob = LocalBoxStream<'static, Result<VideoStep, PortFailure>>;

/// What the product does for the plugins' port: the operations that reach
/// a conversation's Hosts through its host access, and the user's blobs.
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

    /// Reads `reads` where `on` says in the form that never wakes the Host;
    /// [`PortRefusal::NotRunning`] when it is not running, or, on the
    /// primary Host of a device, when it is no longer that device's.
    fn read_host_files<'a>(
        &'a self,
        conversation: &'a ConversationId,
        on: &'a ReadOn,
        reads: Vec<HostRead>,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<Vec<HostFile>, PortFailure>>;

    /// The models of `kind` of the user's provider entries, each with the
    /// entries that offer it.
    fn media_models(&self, kind: MediaKind) -> LocalBoxFuture<'_, Result<MediaListing, PortFailure>>;

    /// Makes the image `ask` names for the conversation, through the entry
    /// and account the providers choose; its usage ledger row names the
    /// agent and the turn of `caller`, the job that called the command, if
    /// any.
    fn make_image<'a>(
        &'a self,
        conversation: &'a ConversationId,
        caller: Option<&'a JobCaller>,
        ask: ImageAsk,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<MadeImage, PortFailure>>;

    /// Asks for the video `ask` names for the conversation, through the
    /// entry and account the providers choose, and answers the job the
    /// vendor took with its steps; its usage ledger row names the agent and
    /// the turn of `caller`. Dropping the steps before the video ends the
    /// job's waiting and cancels it where the vendor can.
    fn make_video<'a>(
        &'a self,
        conversation: &'a ConversationId,
        caller: Option<&'a JobCaller>,
        ask: VideoAsk,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<(AskedVideo, VideoJob), PortFailure>>;

    /// Makes the audio `ask` names for the conversation, through the entry
    /// the providers choose; its usage ledger row names the agent and the
    /// turn of `caller`.
    fn make_audio<'a>(
        &'a self,
        conversation: &'a ConversationId,
        caller: Option<&'a JobCaller>,
        ask: AudioAsk,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<MadeAudio, PortFailure>>;

    /// The voices of speech model `model`, as the entry a call of it takes
    /// lists them, narrowed by `search`.
    fn audio_voices<'a>(
        &'a self,
        conversation: &'a ConversationId,
        model: String,
        search: Option<String>,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<AudioVoices, PortFailure>>;

    /// Stores `bytes` in the user's blob namespace.
    fn put_blob(&self, bytes: B64Bytes) -> LocalBoxFuture<'_, Result<BlobRef, PortFailure>>;

    fn get_blob(&self, blob: BlobRef) -> LocalBoxFuture<'_, Result<Option<B64Bytes>, PortFailure>>;

    /// `plaintext` sealed as `user`'s secret `name` of `plugin`, by the one
    /// owner of sealing (`crates/backend/src/config/secret.rs`).
    fn seal(&self, user: &UserId, plugin: &str, name: &str, plaintext: &[u8]) -> Vec<u8>;

    /// What `sealed` holds as that secret; none when it does not open.
    fn open(&self, user: &UserId, plugin: &str, name: &str, sealed: &[u8]) -> Option<Vec<u8>>;

    /// The client every outbound request of the plugins goes through: the shard's, which sends only
    /// `https` and follows no redirect away from it.
    fn web_client(&self) -> reqwest::Client;

    /// The backend's public URL, which an OAuth redirect URI starts with;
    /// none before the backend listens.
    fn public_url(&self) -> Option<Url>;

    /// The wall clock.
    fn now(&self) -> Timestamp;

    /// Runs `work` as a task of the user's shard, which the shard's close
    /// waits for: work that runs to its end though whoever started it went
    /// away, such as a token renewal that asked its vendor. A shard already
    /// gone drops `work` unstarted.
    fn spawn(&self, work: LocalBoxFuture<'static, ()>);
}

/// Where a request's reads of files go (`crates/plugin-interface/src/lib.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOn {
    /// The conversation's primary Host.
    Primary,
    /// The primary Host, while it is this device's, as a project search
    /// asked of it.
    PrimaryOf(DeviceId),
    /// A command's job: the Host it runs on, the conversation's primary
    /// Host or an attached one, with a relative path read against the
    /// job's directory.
    Job { device: DeviceId, directory: String },
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

/// Why a change of a work panel was refused.
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

/// What follows from a plugin turned on or off that the product does.
#[derive(Debug)]
pub struct Switched {
    /// The packages whose services every connected Host stops, which ends
    /// what they hold for every conversation: those of a plugin turned off
    /// that no plugin still on binds.
    pub released: Vec<String>,
}

/// The commands and revision of the plugins a user has on, which a
/// conversation's tree opens with.
pub struct PluginToolset {
    pub commands: CommandSet,
    /// The ids of the plugins on that declare commands, in registration
    /// order: two sets of the same commands have the same revision.
    pub revision: String,
}

/// A plugin's instance while the user has it on.
#[derive(Clone)]
struct Live {
    instance: Rc<dyn Plugin>,
    /// Cancelled when the user turns the plugin off, which ends its requests
    /// under way and its open user streams.
    ended: CancellationToken,
}

/// What every request of the user's plugins shares.
pub(crate) struct Shared {
    pub(crate) registry: Arc<Registry>,
    pub(crate) user: UserId,
    /// The instance of each plugin the user has on, by its index, once a
    /// request made it.
    lives: RefCell<Vec<Option<Live>>>,
    pub(crate) product: Rc<dyn ProductPort>,
    pub(crate) control: ControlService,
    /// The user's OAuth sign-ins under way, and the turns of their renewals.
    pub(crate) sign_ins: SignIns,
    marks: UserMarks,
    /// Whether the user has each plugin on, by its index, once read: every
    /// switch goes through this shard, so the copy stays current.
    enabled: RefCell<Option<Vec<bool>>>,
    /// Cancelled when the user next turns a plugin on or off, which makes
    /// what a direct channel's introduction told of the user's streams out
    /// of date (`crates/runner-direct/src/lib.rs`).
    switched: RefCell<CancellationToken>,
    /// Each plugin's Host directories for the user, by its index, once
    /// read: every change goes through this shard.
    directories: RefCell<Option<Vec<Vec<HostDirectory>>>>,
    /// How many times each plugin's conversation state changed since the
    /// shard started, by the plugin's index and the conversation; a pair
    /// without an entry has changed none.
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
        self.request_on(plugin, request, conversation, None, rpc, cancel).await
    }

    /// [`Shared::request`], whose reads of the conversation's files are
    /// bound to the primary Host of `host` when one is named.
    pub(crate) async fn request_on(
        self: &Rc<Self>,
        plugin: usize,
        request: Request,
        conversation: Option<ConversationId>,
        host: Option<DeviceId>,
        rpc: Option<RpcPort>,
        cancel: CancellationToken,
    ) -> Result<Reply, PluginError> {
        let (caller, read_on) = match (&request, host) {
            (Request::Command { invocation, .. }, _) => {
                let caller = invocation.caller.clone();
                // A Host name that is no device's reads nowhere; the read
                // answers that the Host is not running.
                let read_on = match DeviceId::try_from(invocation.host.as_str()) {
                    Ok(device) => ReadOn::Job { device, directory: invocation.cwd.clone() },
                    Err(_) => ReadOn::Primary,
                };
                (caller, read_on)
            }
            (_, Some(device)) => (None, ReadOn::PrimaryOf(device)),
            (_, None) => (None, ReadOn::Primary),
        };
        let transport = RequestPort {
            shared: self.clone(),
            plugin,
            conversation,
            rpc,
            cancel: cancel.clone(),
            read_on,
            caller,
            video: RefCell::new(None),
        };
        let enabled = self.enabled().await.map_err(PluginError::failed)?;
        let Some(live) = self.live(plugin, &enabled) else {
            return Err(PluginError::Ended {
                message: format!("The plugin \"{}\" is off", self.plugin_id(plugin)),
            });
        };
        let port = PluginPort::new(Rc::new(transport), cancel);
        // A request the switch cut off ends here, which drops what it was
        // doing; the instance goes once the last request holding it ended.
        tokio::select! {
            reply = live.instance.call(request, port) => reply,
            () = live.ended.cancelled() => Err(PluginError::Ended {
                message: format!("The plugin \"{}\" was turned off", self.plugin_id(plugin)),
            }),
        }
    }

    /// The instance of plugin `plugin`, made now when it has none; none
    /// while `enabled`, the user's switches, has it off.
    fn live(&self, plugin: usize, enabled: &[bool]) -> Option<Live> {
        if !enabled[plugin] {
            return None;
        }
        let mut lives = self.lives.borrow_mut();
        let live = lives[plugin].get_or_insert_with(|| Live {
            instance: self.registry.plugins[plugin].factory.instance(),
            ended: CancellationToken::new(),
        });
        Some(live.clone())
    }

    /// Whether the user has each plugin on, by its index.
    async fn enabled(&self) -> Result<Vec<bool>, StorageError> {
        if let Some(enabled) = &*self.enabled.borrow() {
            return Ok(enabled.clone());
        }
        let choices = self.control.user_plugins(self.user.clone()).await?;
        let enabled: Vec<bool> = self
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
        self.enabled.replace(Some(enabled.clone()));
        Ok(enabled)
    }

    pub(crate) fn plugin_id(&self, plugin: usize) -> String {
        self.registry.plugins[plugin].id().as_str().to_owned()
    }

    fn revision(&self, plugin: usize, conversation: &ConversationId) -> u64 {
        let key = (plugin, conversation.clone());
        self.revisions.borrow().get(&key).copied().unwrap_or(0)
    }

    /// Marks plugin `plugin`'s page state of `scope` changed: the user's
    /// goes to every page of the user; a conversation's raises its revision,
    /// which the conversation's metadata carries.
    pub(crate) fn changed(&self, plugin: usize, conversation: Option<&ConversationId>) {
        match conversation {
            None => self.marks.mark(Part::Plugin(self.plugin_id(plugin))),
            Some(conversation) => {
                let key = (plugin, conversation.clone());
                *self.revisions.borrow_mut().entry(key).or_default() += 1;
                self.marks.mark(Part::Metadata(conversation.clone()));
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
                self.marks.mark(Part::Metadata(conversation.clone()));
                Ok((revision, effects))
            }
            PanelOutcome::Unchanged { revision } => Ok((revision, Vec::new())),
            PanelOutcome::Full => Err(PanelError::Full),
            PanelOutcome::TooLarge => Err(PanelError::TooLarge),
            PanelOutcome::Archived => Err(PanelError::Archived),
            PanelOutcome::Missing => Err(PanelError::Missing),
        }
    }

    /// Tells the plugin that owns the kind of the tab the user removed, if the
    /// user has it on, after the change is answered: the page never waits for
    /// the plugin's work, which may wake a Host.
    ///
    /// A tab the user created is none of the plugin's business: the user
    /// creates tabs of the kinds no plugin owns, such as a file's, and the
    /// plugin its own.
    fn tell_owner(
        self: &Rc<Self>,
        conversation: ConversationId,
        effect: PanelEffect,
        enabled: &[bool],
    ) {
        let PanelEffect::Removed(tab) = effect else {
            return;
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
            )
            .await
            .map_err(storage)?;
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

/// What a package lost on a Host (`crates/command-protocol/src/messages.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lost<'a> {
    /// The state of one package, whose service ended unsaid.
    Package(&'a str),
    /// The state of every package, which a runner that started anew or a
    /// Cloud that stopped took with it.
    Every,
}

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
        let lives = registry.plugins.iter().map(|_| None).collect();
        let sign_ins = SignIns::new(registry.plugins.len());
        Self(Rc::new(Shared {
            registry,
            user,
            lives: RefCell::new(lives),
            product,
            control,
            sign_ins,
            marks,
            enabled: RefCell::new(None),
            switched: RefCell::default(),
            directories: RefCell::new(None),
            revisions: RefCell::new(HashMap::new()),
        }))
    }

    /// Whether the user has each plugin on, by its index.
    async fn enabled(&self) -> Result<Vec<bool>, StorageError> {
        self.0.enabled().await
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
    /// the plugin's state as changed; any change ends what
    /// [`Self::next_switch`] gave. Answers what the product does next, or none
    /// when the choice did not change.
    ///
    /// A plugin turned off loses its instance, which ends its requests under
    /// way, the tasks it started and its open user streams, and the tabs of
    /// its kinds in every conversation's panel, an archived one's included.
    /// Its durable data stays.
    pub async fn switch(&self, plugin: &str, enabled: bool) -> Result<Option<Switched>, SwitchError> {
        let Some((index, registered)) = self.0.registry.plugin(plugin) else {
            return Err(SwitchError::UnknownPlugin(plugin.to_owned()));
        };
        let mut current = self.enabled().await?;
        if current[index] == enabled {
            return Ok(None);
        }
        self.0
            .control
            .set_user_plugin(self.0.user.clone(), plugin.to_owned(), enabled)
            .await?;
        current[index] = enabled;
        self.0.enabled.replace(Some(current.clone()));
        let mut released = Vec::new();
        if !enabled {
            if let Some(live) = self.0.lives.borrow_mut()[index].take() {
                live.ended.cancel();
            }
            self.close_tabs(index).await?;
            // A package another plugin on binds stays; none does today.
            released = registered
                .packages
                .iter()
                .filter(|package| {
                    !self.0.registry.plugins.iter().enumerate().any(|(other, registered)| {
                        current[other] && registered.packages.contains(*package)
                    })
                })
                .cloned()
                .collect();
        }
        let switched = std::mem::take(&mut *self.0.switched.borrow_mut());
        switched.cancel();
        self.0.marks.mark(Part::Plugins);
        self.0
            .marks
            .mark(Part::Plugin(registered.id().as_str().to_owned()));
        Ok(Some(Switched { released }))
    }

    /// Removes the tabs of plugin `plugin`'s kinds from every panel of the
    /// user's conversations, archived ones included, and marks the summary
    /// of each that changed.
    async fn close_tabs(&self, plugin: usize) -> Result<(), StorageError> {
        let Some(page) = &self.0.registry.plugins[plugin].page else {
            return Ok(());
        };
        let closed = self
            .0
            .control
            .close_panel_kinds(self.0.user.clone(), page.panel_kinds.clone())
            .await?;
        for conversation in closed {
            self.0.marks.mark(Part::Metadata(conversation));
        }
        Ok(())
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
            paths: asked.paths,
            turn: asked.turn,
            seen: asked.seen,
            project: asked.project,
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

    /// Whether `plugin` declares a project search and the user has it on.
    pub async fn searches_projects(&self, plugin: &PluginId) -> Result<bool, StorageError> {
        let Some((index, registered)) = self.0.registry.plugin(plugin.as_str()) else {
            return Ok(false);
        };
        Ok(registered.factory.manifest().project_search && self.enabled().await?[index])
    }

    /// Asks `plugin`, which declares a project search, to search
    /// `directory`, the conversation's project directory on the primary Host
    /// of `device`, which its reads are bound to: what it found and what
    /// tells of a change to it; none while the user has it off.
    pub async fn project_search(
        &self,
        plugin: &PluginId,
        conversation: &ConversationId,
        device: &DeviceId,
        directory: String,
        paths: demi_host_interface::HostPaths,
        cancel: CancellationToken,
    ) -> Result<Option<(Value, Vec<WatchSubject>)>, PluginError> {
        let Some((index, _)) = self.0.registry.plugin(plugin.as_str()) else {
            return Ok(None);
        };
        let enabled = self.enabled().await.map_err(PluginError::failed)?;
        if !enabled[index] {
            return Ok(None);
        }
        let request = Request::ProjectSearch {
            user: self.0.user.clone(),
            conversation: conversation.clone(),
            directory,
            paths,
        };
        let reply = self
            .0
            .request_on(index, request, Some(conversation.clone()), Some(device.clone()), None, cancel)
            .await?;
        match reply {
            Reply::Project { found, watch } => Ok(Some((found, watch))),
            reply => Err(PluginError::failed(format!(
                "the plugin answered a project search with {reply:?}"
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

    /// What is cancelled when the user next turns a plugin on or off. Taken
    /// before the user's streams are read, it tells that what was read is
    /// out of date, such as a direct channel's introduction.
    pub fn next_switch(&self) -> CancellationToken {
        self.0.switched.borrow().clone()
    }

    /// What ends the user stream `name` once its plugin is turned off; none
    /// while the user has its plugin off, when the stream does not exist.
    pub async fn stream_end(&self, name: &str) -> Result<Option<CancellationToken>, StorageError> {
        let Some(index) = self.0.registry.stream_owner(name) else {
            return Ok(None);
        };
        let enabled = self.enabled().await?;
        Ok(self.0.live(index, &enabled).map(|live| live.ended))
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

    /// The command package `package` said `message` about `conversation`
    /// from a service on one of its Hosts: each plugin that binds it and
    /// that the user has on receives it, in the order the messages came
    /// (`crates/plugin-interface/src/request.rs`).
    pub async fn package_message(
        &self,
        package: &str,
        conversation: &ConversationId,
        message: serde_json::Value,
    ) {
        for plugin in self.receivers(package).await {
            let request = Request::PackageMessage {
                user: self.0.user.clone(),
                conversation: conversation.clone(),
                message: message.clone(),
            };
            self.0.spawn_request(plugin, request, conversation.clone());
        }
    }

    /// What `lost` names was kept on a Host of each of `conversations` and
    /// ended without its package saying so.
    ///
    /// Each plugin that binds such a package, that the user has on and that has
    /// panel tabs in one of the conversations, receives `package_lost` for it
    /// (`crates/command-protocol/src/messages.rs`). A plugin that shows nothing
    /// of a conversation has nothing to correct.
    pub async fn package_lost(&self, lost: Lost<'_>, conversations: &[ConversationId]) {
        let plugins = match lost {
            Lost::Package(package) => self.receivers(package).await,
            Lost::Every => self.receivers_of(|registered| !registered.packages.is_empty()).await,
        };
        if plugins.is_empty() {
            return;
        }
        for conversation in conversations {
            let panel = match self.0.control.panel(conversation.clone()).await {
                Ok(panel) => panel,
                Err(error) => {
                    // The plugin then shows the lost state until its
                    // package next says something about the conversation.
                    tracing::warn!(%conversation, error = &error as &dyn std::error::Error, "a lost package's panel could not be read");
                    continue;
                }
            };
            for &plugin in &plugins {
                let registered = &self.0.registry.plugins[plugin];
                if !panel.tabs.iter().any(|tab| registered.owns_kind(&tab.kind)) {
                    continue;
                }
                let request = Request::PackageLost {
                    user: self.0.user.clone(),
                    conversation: conversation.clone(),
                };
                self.0.spawn_request(plugin, request, conversation.clone());
            }
        }
    }

    /// The plugins that bind `package` and that the user has on.
    async fn receivers(&self, package: &str) -> Vec<usize> {
        self.receivers_of(|registered| registered.packages.contains(package)).await
    }

    /// The plugins `bound` takes that the user has on, by their indices.
    async fn receivers_of(&self, bound: impl Fn(&crate::registry::Registered) -> bool) -> Vec<usize> {
        let enabled = match self.enabled().await {
            Ok(enabled) => enabled,
            Err(error) => {
                tracing::warn!(%error, "the plugins a package message is for could not be read");
                return Vec::new();
            }
        };
        self.0
            .registry
            .plugins
            .iter()
            .enumerate()
            .filter(|(index, registered)| enabled[*index] && bound(registered))
            .map(|(index, _)| index)
            .collect()
    }

    /// The conversation's work panel.
    pub async fn panel(&self, conversation: ConversationId) -> Result<WorkPanel, StorageError> {
        self.0.control.panel(conversation).await
    }

    /// The conversation's work panel with the held states of the plugins the
    /// user has on whose conversation state is held, each as its plugin holds
    /// it now: a plugin that has none, or fails, adds nothing, and the page
    /// reads that state itself.
    pub async fn panel_read(
        &self,
        conversation: ConversationId,
        cancel: CancellationToken,
    ) -> Result<PanelRead, StorageError> {
        let panel = self.0.control.panel(conversation.clone()).await?;
        let enabled = self.enabled().await?;
        let mut states = Vec::new();
        for (index, registered) in self.0.registry.plugins.iter().enumerate() {
            let held = registered.state(Scope::Conversation).is_some_and(|state| state.held);
            if !enabled[index] || !held {
                continue;
            }
            // Read after the revision, as `conversation_state` does.
            let revision = self.0.revision(index, &conversation);
            let request = Request::PageState {
                user: self.0.user.clone(),
                conversation: Some(conversation.clone()),
                held: true,
            };
            let reply = self
                .0
                .request(index, request, Some(conversation.clone()), None, cancel.clone())
                .await;
            if let Ok(Reply::State { state }) = reply {
                states.push(HeldState {
                    plugin: registered.id().as_str().to_owned(),
                    revision,
                    state,
                });
            }
        }
        Ok(PanelRead {
            revision: panel.revision,
            tabs: panel.tabs,
            states,
        })
    }

    /// A page's changes of the conversation's work panel, all or none: each tab
    /// they create is of a kind a plugin the user has on declares.
    ///
    /// Answers the panel's revision once the changes are in it and whether they
    /// changed it, and tells the kind's plugin of each tab the user created or
    /// removed.
    pub async fn change_panel(
        &self,
        conversation: ConversationId,
        changes: Vec<PanelChange>,
    ) -> Result<(u64, bool), PanelError> {
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
        let changed = !effects.is_empty();
        for effect in effects {
            self.0.tell_owner(conversation.clone(), effect, &enabled);
        }
        Ok((revision, changed))
    }

    /// The vendor sent the browser back to `plugin`'s OAuth callback: a sign-in this user started,
    /// of a plugin the user has on, is completed; answers the vendor's name.
    pub async fn oauth_callback(
        &self,
        plugin: &str,
        callback: Callback,
        cancel: CancellationToken,
    ) -> Result<String, CallbackError> {
        let unknown = || CallbackError::UnknownPlugin(plugin.to_owned());
        let Some((index, _)) = self.0.registry.plugin(plugin) else {
            return Err(unknown());
        };
        if !self.enabled().await?[index] {
            return Err(unknown());
        }
        self.0.oauth_callback(index, callback, &cancel).await
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
            held: false,
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
    /// How the node's Host writes its paths.
    pub paths: demi_host_interface::HostPaths,
    /// The node's current input turn.
    pub turn: TurnId,
    /// The text of the source's own blocks the model receives, oldest
    /// first.
    pub seen: Vec<String>,
    /// What the plugin's project search last found, for a plugin that
    /// declares one.
    pub project: Option<Value>,
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
    /// Where its reads of files go.
    read_on: ReadOn,
    /// The job that called the command, for a command an agent's job
    /// called.
    caller: Option<JobCaller>,
    /// The request's video's job, from its ask to its video; dropped with
    /// the request.
    video: RefCell<Option<VideoJob>>,
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
                    .write_plugin_value(write)
                    .await
                    .map_err(storage)?;
                match written {
                    Written::Revision(revision) => PortAnswer::Written { revision },
                    Written::Conflict => return Err(PortFailure::Refused(PortRefusal::Conflict)),
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
                    )
                    .await
                    .map_err(storage)?;
                match removed {
                    Written::Revision(_) => PortAnswer::Done,
                    Written::Conflict => return Err(PortFailure::Refused(PortRefusal::Conflict)),
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
                    .read_host_files(self.conversation()?, &self.read_on, reads, &self.cancel)
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
            PortMessage::ReadSecret { name } => PortAnswer::Secret {
                secret: shared.read_secret(self.plugin, name.as_str()).await?,
            },
            PortMessage::WriteSecret {
                name,
                value,
                revision,
            } => PortAnswer::Written {
                revision: shared
                    .write_secret(self.plugin, name.as_str(), &value, revision)
                    .await?,
            },
            PortMessage::RemoveSecret { name, revision } => {
                shared
                    .remove_secret(self.plugin, name.as_str(), revision)
                    .await?;
                PortAnswer::Done
            }
            PortMessage::SendRequest { request } => {
                let id = shared.registry.plugins[self.plugin].id();
                PortAnswer::Response {
                    response: web::send(&product.web_client(), id, request, &self.cancel).await?,
                }
            }
            PortMessage::OauthStart => PortAnswer::AuthorizationUrl {
                url: shared.oauth_start(self.plugin, &self.cancel).await?,
            },
            PortMessage::OauthToken { refused } => PortAnswer::AccessToken {
                token: shared
                    .oauth_token(self.plugin, refused.as_deref(), &self.cancel)
                    .await?,
            },
            PortMessage::OauthSignOut => {
                shared.oauth_sign_out(self.plugin, &self.cancel).await?;
                PortAnswer::Done
            }
            PortMessage::MediaModels { kind } => PortAnswer::MediaModels {
                listing: product.media_models(kind).await?,
            },
            PortMessage::MakeImage { ask } => PortAnswer::Image {
                image: product
                    .make_image(self.conversation()?, self.caller.as_ref(), ask, &self.cancel)
                    .await?,
            },
            PortMessage::MakeVideo { ask } => {
                if self.video.borrow().is_some() {
                    return Err(Self::job_failure("the call has a video's job already"));
                }
                let (asked, job) = product
                    .make_video(self.conversation()?, self.caller.as_ref(), ask, &self.cancel)
                    .await?;
                self.video.replace(Some(job));
                PortAnswer::VideoAsked { asked }
            }
            PortMessage::MakeAudio { ask } => PortAnswer::Audio {
                audio: product
                    .make_audio(self.conversation()?, self.caller.as_ref(), ask, &self.cancel)
                    .await?,
            },
            PortMessage::AudioVoices { model, search } => PortAnswer::AudioVoices {
                voices: product.audio_voices(self.conversation()?, model, search, &self.cancel).await?,
            },
            PortMessage::VideoProgress => {
                // Taken out for the wait, so no borrow is held across it.
                let Some(mut job) = self.video.take() else {
                    return Err(Self::job_failure("the call has no video's job"));
                };
                let step = tokio::select! {
                    step = job.next() => step,
                    () = self.cancel.cancelled() => return Err(PortError::Ended("the call was stopped".into()).into()),
                };
                match step {
                    Some(Ok(step)) => {
                        // The job ends with its video; until then it waits
                        // for the next ask.
                        if !matches!(step, VideoStep::Made { .. }) {
                            self.video.replace(Some(job));
                        }
                        PortAnswer::VideoProgress { step }
                    }
                    Some(Err(failure)) => return Err(failure),
                    None => {
                        return Err(Self::job_failure("the video's job ended"));
                    }
                }
            }
        };
        Ok(answer)
    }

    /// Why a step of a video's job was not answered.
fn job_failure(message: &str) -> PortFailure {
    PortFailure::Refused(PortRefusal::Media { refusal: MediaRefusal::Failed { message: message.into() } })
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
pub(crate) fn storage(error: StorageError) -> PortFailure {
    PortFailure::Port(PortError::Failed(error.to_string()))
}
