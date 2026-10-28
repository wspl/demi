//! The `skills` plugin.
//!
//! The plugin holds:
//!
//! - the user's skill sources, fetched from git and pinned to a commit, whose
//!   skills the user turns on one by one and which the Hosts that run the
//!   conversations offered them receive as directories;
//! - the skills a repository carries;
//! - the catalog of both, which reaches the model as a context block.
//!
//! Nothing else in Demi knows skills.
//!
//! # Places
//!
//! A source is in one place, which the user picks when adding it:
//!
//! ```text
//!   Global ───────────┬──► every conversation's catalog
//!   workspace Work ───┴──► Work's conversations' catalog only
//! ```
//!
//! A repository is a source in one place, since a source's id is its
//! repository's. A workspace's deletion takes its sources with it: the next
//! read of the sources removes those of a workspace that no longer exists.
//!
//! # Which skills are on
//!
//! A source never changes by itself: its skills change only when the user adds,
//! updates or removes it or turns its skills on or off.
//!
//! A new skill starts on, with two exceptions:
//!
//! - A skill whose name another skill that is on has.
//! - A skill that sets `disable-model-invocation`, which the agent is never
//!   offered. On, it would only install its files on every Host and take its
//!   name from another skill.
//!
//! Two skills that are on never share a name, in any place, which an update
//! cannot break either, since a skill stays on only under the name it was on
//! with: a Host may run conversations of several workspaces, and a skill's
//! directory is named after it.
//!
//! # Hosts
//!
//! A Global skill that is on is installed on each Host the user's jobs run
//! on, and a workspace's on each Host its conversations' jobs run on, so the
//! bounds of a fetch bound what a Host keeps.
//!
//! A skill's path in the catalog is its directory on a Host, the same on every
//! Host, so a target switch changes no path: the new Host receives the
//! directory when its first job needs it.
//!
//! The plugin writes to no Host and never blocks the shard thread.
//!
//! # Fetchers
//!
//! The server fetches a source with gix on the blocking pool, and sends no
//! credential. A source may instead name one of the user's paired devices,
//! which fetches it with its own `git`, as its user, with that user's keys,
//! credential helpers and network (`crates/backend-host-access/src/device_git.rs`):
//! the one way to an SSH address, or a repository only a company network
//! reaches. What either fetch answers is read by the same rules, and the
//! server keeps the files for every Host. An offline device fetches and
//! checks nothing; its source keeps its skills and waits.
//!
//! # Updates
//!
//! The backend checks each source the server fetches on its own schedule, once
//! per interval however many users added the source (`heads.rs`), from the
//! first page state a user's instance reads until the instance ends, and tells
//! those users' pages when it moved. Opening Settings › Skills checks nothing.
//!
//! A source a device fetches is checked by that device, on the same
//! schedule, while it is online. What it finds is this instance's alone, so
//! a private repository's commit reaches no other user.

mod catalog;
mod commands;
mod fetch;
mod heads;
mod origin;
mod project;
mod skill;
mod sources;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use demi_plugin_interface::{
    CommandPlugin, Manifest, Method, Page, Plugin, PluginError, PluginId, PluginPort, Reply,
    Request, Scope,
};
use demi_shared_types::{Clock, SKILLS_SOURCE, SystemClock};
use futures_util::future::{LocalBoxFuture, join_all};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use tokio::sync::{broadcast, watch};
use tokio::task::AbortHandle;
use tracing::Instrument as _;

use crate::catalog::{Entry, NONE_AVAILABLE};
use crate::fetch::FetchError;
use crate::heads::{CHECK_INTERVAL, Heads};
use crate::origin::Origin;
use crate::project::ProjectSkill;
use crate::sources::{Places, Source, Sources};
use demi_web_api_protocol::ids::{DeviceId, WorkspaceId};
use demi_plugin_interface::{GitRefusal, PortFailure, PortRefusal};

pub use crate::fetch::Skipped;
pub use crate::sources::{Failure, SkillState, SkillsState, SourceState};

/// Maps a source's URL to the URL its fetch reads: the URL itself, or in a
/// test, a repository the test made.
pub type Resolve = dyn Fn(&str) -> String + Send + Sync;

/// The plugin's factory.
pub struct Skills {
    manifest: Manifest,
    resolve: Arc<Resolve>,
    clock: Arc<dyn Clock>,
    /// Every source's head, which every user's instance shares.
    heads: Arc<Heads>,
}

impl Skills {
    pub fn new() -> Self {
        Self::resolving(Arc::new(str::to_owned), Arc::new(SystemClock))
    }

    /// A factory whose fetches read `resolve`'s URL for each source's, with
    /// `clock` telling when a fetch ended: a test's repositories and time.
    pub fn resolving(resolve: Arc<Resolve>, clock: Arc<dyn Clock>) -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from(SKILLS_SOURCE).expect("a valid plugin id"),
            "Skills",
            "Teaches the agent workflows with skills from Git repositories you add with demi skills, and from your repository.",
        );
        manifest.context = true;
        manifest.project_search = true;
        manifest.commands = commands::commands().manifest_commands();
        manifest.page = Some(page());
        Self {
            manifest,
            resolve,
            clock,
            heads: Arc::default(),
        }
    }
}

impl Default for Skills {
    fn default() -> Self {
        Self::new()
    }
}

impl demi_plugin_interface::PluginFactory for Skills {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(Instance(Rc::new(State {
            commands: commands::commands(),
            plugin: self.manifest.id.clone(),
            resolve: self.resolve.clone(),
            clock: self.clock.clone(),
            fetching: RefCell::default(),
            fetches: RefCell::default(),
            heads: self.heads.clone(),
            device_heads: Heads::default(),
            follow: RefCell::default(),
        })))
    }
}

/// `add_source { origin, workspace, fetcher }`: the source goes to the
/// workspace named, or to Global without one, and the device named fetches
/// it, or the server without one.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddSource {
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetcher: Option<DeviceId>,
}

/// `set_fetcher { source, fetcher }`: the device named fetches the source
/// from now on, or the server without one; it fetches it again at once.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetFetcher {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetcher: Option<DeviceId>,
}

/// What `add_source` answers: the new source's id.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AddedSource {
    pub source: String,
}

/// `update_source { source }` and `remove_source { source }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceCall {
    pub source: String,
}

/// `set_enabled { source, skill, enabled }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetEnabled {
    pub source: String,
    pub skill: String,
    pub enabled: bool,
}

/// `set_source_enabled { source, enabled }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetSourceEnabled {
    pub source: String,
    pub enabled: bool,
}

fn page() -> Page {
    Page::new("@demicodes/plugin-skills")
        .user_state(demi_plugin_interface::State::new::<SkillsState>())
        .method(Method::new::<AddSource, AddedSource>(
            "add_source",
            Scope::User,
        ))
        .method(Method::new::<SourceCall, ()>("update_source", Scope::User))
        .method(Method::new::<SetFetcher, ()>("set_fetcher", Scope::User))
        .method(Method::new::<SourceCall, ()>("remove_source", Scope::User))
        .method(Method::new::<SetEnabled, ()>("set_enabled", Scope::User))
        .method(Method::new::<SetSourceEnabled, ()>(
            "set_source_enabled",
            Scope::User,
        ))
}

/// One user's skills plugin.
struct Instance(Rc<State>);

/// What the instance keeps in memory, for its user only.
struct State {
    /// The `demi skills` group, which the instance answers.
    commands: CommandPlugin,
    plugin: PluginId,
    resolve: Arc<Resolve>,
    clock: Arc<dyn Clock>,
    /// The sources being fetched, by id, each with its fetch's end.
    fetching: RefCell<BTreeMap<String, FetchEnd>>,
    /// Each running fetch: its task and what stops its blocking part.
    fetches: RefCell<Vec<(AbortHandle, Arc<AtomicBool>)>>,
    /// What each source's default branch points to, by id, as every
    /// user's instance knows it: the sources the server fetches.
    heads: Arc<Heads>,
    /// The same of the sources the user's devices fetch, which this
    /// instance alone knows: a private repository's commit reaches no other
    /// user.
    device_heads: Heads,
    /// The task that checks the user's sources on the backend's schedule
    /// and tells the pages of a move, from the first page state read on.
    follow: RefCell<Option<AbortHandle>>,
}

/// How a fetch of a source ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fetched {
    /// The source is pinned to the newest commit.
    Pinned,
    /// The fetch failed, as the source now records.
    Failed(String),
    /// The source was removed while it was fetched.
    Removed,
    /// The shutdown cut the fetch off, or its device went away, which
    /// recorded nothing.
    Stopped,
    /// Another fetcher took the source over while it was fetched: nothing
    /// was recorded, and the new fetcher fetches it next.
    Superseded,
}

/// A running fetch's end, which each command waiting for it reads: none
/// until it ends, then how, or why its recording failed.
type FetchEnd = watch::Receiver<Option<Result<Fetched, String>>>;

/// The instance ends with its shard: every fetch stops and saves nothing.
impl Drop for Instance {
    fn drop(&mut self) {
        for (task, stop) in self.0.fetches.take() {
            stop.store(true, Ordering::Relaxed);
            task.abort();
        }
        // A check's blocking part only reads the remote's refs; it ends on
        // its own, and its result goes nowhere.
        if let Some(task) = self.0.follow.take() {
            task.abort();
        }
    }
}

impl Plugin for Instance {
    fn call(
        &self,
        request: Request,
        port: PluginPort,
    ) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            match request {
                Request::Context { conversation, seen, project, .. } => {
                    let span = tracing::info_span!("skills_context", conversation = %conversation);
                    let text = self.0.context(&port, project, &seen).instrument(span).await?;
                    Ok(Reply::Context { text })
                }
                Request::ProjectSearch {
                    conversation, directory, paths, ..
                } => {
                    let span = tracing::info_span!("skills_search", conversation = %conversation);
                    let (skills, watch) = project::search(&port, paths, &directory).instrument(span).await?;
                    let found = serde_json::to_value(skills).expect("project skills encode as JSON");
                    Ok(Reply::Project { found, watch })
                }
                Request::PageState { .. } => {
                    self.0.follow(&port);
                    let state = self.0.state(&port).await?;
                    Ok(Reply::State {
                        state: serde_json::to_value(state).expect("the state encodes as JSON"),
                    })
                }
                Request::PageCall { method, params, .. } => Ok(Reply::Result {
                    result: self.0.call(&method, params, port).await?,
                }),
                Request::Command { invocation, .. } => {
                    self.0.commands.check(&invocation)?;
                    let code = commands::run(&self.0, *invocation, &port).await?;
                    Ok(Reply::Exit { code })
                }
                Request::PanelTab { .. } => Err(PluginError::undeclared("panel kind")),
                Request::PackageMessage { .. } | Request::PackageLost { .. } => {
                    Err(PluginError::undeclared("package messages"))
                }
            }
        })
    }
}

impl State {
    async fn call(
        self: &Rc<Self>,
        method: &str,
        params: Map<String, Value>,
        port: PluginPort,
    ) -> Result<Value, PluginError> {
        match method {
            "add_source" => {
                let AddSource { origin, workspace, fetcher } = decode(params)?;
                let source = self.add_source(&port, &origin, workspace, fetcher).await?;
                self.start_fetch(port, source.clone());
                Ok(serde_json::to_value(AddedSource { source }).expect("a result encodes"))
            }
            "update_source" => {
                let SourceCall { source } = decode(params)?;
                self.ready_to_fetch(&port, &source).await?;
                self.start_fetch(port, source);
                Ok(Value::Null)
            }
            "set_fetcher" => {
                let SetFetcher { source, fetcher } = decode(params)?;
                self.set_fetcher(&port, &source, fetcher).await?;
                self.start_fetch(port, source);
                Ok(Value::Null)
            }
            "remove_source" => {
                let SourceCall { source } = decode(params)?;
                self.remove_source(&port, &source).await?;
                Ok(Value::Null)
            }
            "set_enabled" => {
                let SetEnabled {
                    source,
                    skill,
                    enabled,
                } = decode(params)?;
                self.switch(&port, &source, |_| BTreeSet::from([skill.clone()]), enabled)
                    .await?;
                Ok(Value::Null)
            }
            "set_source_enabled" => {
                let SetSourceEnabled { source, enabled } = decode(params)?;
                let every = |source: &Source| {
                    source
                        .skills
                        .iter()
                        .map(|skill| skill.name.clone())
                        .collect()
                };
                self.switch(&port, &source, every, enabled).await?;
                Ok(Value::Null)
            }
            method => Err(PluginError::failed(format!("no method \"{method}\""))),
        }
    }

    /// The user's sources that are in a place, and the places. A source of a
    /// workspace that no longer exists is removed on the way, with its Host
    /// directories, and the pages hear of it.
    ///
    /// The sources are read before the workspaces: a source is added only to
    /// a workspace that exists, so a workspace made meanwhile never makes its
    /// new source look orphaned.
    async fn sources(&self, port: &PluginPort) -> Result<(Sources, Places), PluginError> {
        let read = sources::read(port).await?;
        let places = Places::new(port.workspaces().await?);
        let (live, gone) = sources::live(read, &places);
        if gone.is_empty() {
            return Ok((live, places));
        }
        for (id, revision) in gone {
            match port.remove_value(&id, revision).await {
                Ok(()) => {}
                // Another write came first; the next read removes it.
                Err(failure) if sources::conflict(&failure) => {}
                Err(failure) => return Err(failure.into()),
            }
        }
        // Read again: a write that came during the removals is in the set.
        let (live, _) = sources::live(sources::read(port).await?, &places);
        port.set_directories(sources::directories(&live)).await?;
        port.changed(Scope::User).await?;
        Ok((live, places))
    }

    /// The page's state of the user's sources.
    async fn state(&self, port: &PluginPort) -> Result<SkillsState, PluginError> {
        let (sources, _) = self.sources(port).await?;
        let fetching = self.fetching.borrow();
        Ok(sources::state(&sources, |id| fetching.contains_key(id), &self.heads_of(&sources)))
    }

    /// The heads of `sources`: the server's from the heads every instance
    /// shares, a device's from this instance's own.
    fn heads_of(&self, sources: &Sources) -> BTreeMap<String, heads::Head> {
        let (by_device, by_server): (Vec<&String>, Vec<&String>) =
            sources.iter().map(|(id, (source, _))| (id, source.fetcher.is_some())).fold(
                (Vec::new(), Vec::new()),
                |(mut device, mut server), (id, on_device)| {
                    if on_device { device.push(id) } else { server.push(id) }
                    (device, server)
                },
            );
        let mut heads = self.heads.of(by_server);
        heads.extend(self.device_heads.of(by_device));
        heads
    }

    /// Refuses to fetch `origin` with `fetcher`: an SSH address the server
    /// would fetch, a device that is not paired, or one that is offline.
    async fn check_fetcher(
        &self,
        port: &PluginPort,
        origin: &Origin,
        fetcher: Option<&DeviceId>,
    ) -> Result<(), PluginError> {
        let Some(fetcher) = fetcher else {
            if origin.ssh() {
                return Err(PluginError::refused(
                    "ssh_needs_device",
                    "An SSH address needs a device to fetch it, with its own keys",
                ));
            }
            return Ok(());
        };
        let devices = port.devices().await?;
        match devices.iter().find(|device| device.id == *fetcher) {
            None => Err(PluginError::refused("fetcher_not_found", "The device that fetches the source is not paired")),
            Some(device) if !device.online => {
                Err(PluginError::refused("fetcher_offline", format!("{} is offline", device.name)))
            }
            Some(_) => Ok(()),
        }
    }

    /// Refuses to fetch source `id` now, as [`check_fetcher`](Self::check_fetcher) does.
    async fn ready_to_fetch(&self, port: &PluginPort, id: &str) -> Result<(), PluginError> {
        let (source, _) = sources::read_one(port, id).await?;
        let origin = Origin::parse(&source.origin)
            .map_err(|message| PluginError::failed(format!("a stored origin: {message}")))?;
        self.check_fetcher(port, &origin, source.fetcher.as_ref()).await
    }

    /// Has `fetcher` fetch source `id` from now on, writing again over
    /// another write that came first; refused while the source is fetched.
    async fn set_fetcher(&self, port: &PluginPort, id: &str, fetcher: Option<DeviceId>) -> Result<(), PluginError> {
        if self.fetching.borrow().contains_key(id) {
            return Err(PluginError::refused(
                "source_fetching",
                "The source is being fetched; choose what fetches it once the fetch ends",
            ));
        }
        loop {
            let (source, revision) = sources::read_one(port, id).await?;
            let origin = Origin::parse(&source.origin)
                .map_err(|message| PluginError::failed(format!("a stored origin: {message}")))?;
            self.check_fetcher(port, &origin, fetcher.as_ref()).await?;
            let changed = Source { fetcher: fetcher.clone(), ..source };
            match sources::write(port, id, &changed, Some(revision)).await {
                Ok(_) => break,
                Err(failure) if sources::conflict(&failure) => continue,
                Err(failure) => return Err(failure.into()),
            }
        }
        port.changed(Scope::User).await?;
        Ok(())
    }

    /// Records the source `origin` names in `workspace`, or in Global
    /// without one, with no commit and no skills, and answers its id. A
    /// repository is a source in one place: one another place has is
    /// refused, naming the place.
    async fn add_source(
        &self,
        port: &PluginPort,
        origin: &str,
        workspace: Option<WorkspaceId>,
        fetcher: Option<DeviceId>,
    ) -> Result<String, PluginError> {
        let parsed = Origin::parse(origin)
            .map_err(|message| PluginError::refused("invalid_origin", message))?;
        self.check_fetcher(port, &parsed, fetcher.as_ref()).await?;
        let id = parsed.id();
        let (existing, places) = self.sources(port).await?;
        if !places.exists(workspace.as_ref()) {
            return Err(PluginError::refused("workspace_not_found", "The workspace no longer exists"));
        }
        let taken = |place: Option<&WorkspaceId>| {
            PluginError::refused(
                "source_exists",
                format!("{} is added already, in {}", parsed.url(), places.name(place)),
            )
        };
        if let Some((other, _)) = existing.get(&id) {
            return Err(taken(other.workspace.as_ref()));
        }
        let added = existing
            .values()
            .map(|(source, _)| source.added)
            .max()
            .unwrap_or(0)
            + 1;
        let source = Source {
            origin: origin.trim().to_owned(),
            added,
            commit: None,
            fetched_at: None,
            skills: Vec::new(),
            skipped: Vec::new(),
            failure: None,
            workspace,
            fetcher,
        };
        match sources::write(port, &id, &source, None).await {
            Ok(_) => {}
            Err(failure) if sources::conflict(&failure) => {
                let other = sources::find(port, &id).await?.and_then(|(other, _)| other.workspace);
                return Err(taken(other.as_ref()));
            }
            Err(failure) => return Err(failure.into()),
        }
        port.changed(Scope::User).await?;
        Ok(id)
    }

    async fn remove_source(&self, port: &PluginPort, id: &str) -> Result<(), PluginError> {
        loop {
            let (_, revision) = sources::read_one(port, id).await?;
            match port.remove_value(id, revision).await {
                Ok(()) => break,
                Err(failure) if sources::conflict(&failure) => continue,
                Err(failure) => return Err(failure.into()),
            }
        }
        self.directories_changed(port).await
    }

    /// Turns the skills `chosen` picks of source `id` on or off, writing
    /// again over another write that came first.
    async fn switch(
        &self,
        port: &PluginPort,
        id: &str,
        chosen: impl Fn(&Source) -> BTreeSet<String>,
        enabled: bool,
    ) -> Result<(), PluginError> {
        loop {
            let (all, places) = self.sources(port).await?;
            let (source, revision) = all.get(id).ok_or_else(|| sources::not_found(id))?;
            let changed = sources::switch(&all, &places, id, &chosen(source), enabled)?;
            match sources::write(port, id, &changed, Some(*revision)).await {
                Ok(_) => break,
                Err(failure) if sources::conflict(&failure) => continue,
                Err(failure) => return Err(failure.into()),
            }
        }
        self.directories_changed(port).await
    }

    /// The skills that are on changed: the Host directories follow, and
    /// every page receives the new state.
    async fn directories_changed(&self, port: &PluginPort) -> Result<(), PluginError> {
        let (all, _) = self.sources(port).await?;
        port.set_directories(sources::directories(&all)).await?;
        port.changed(Scope::User).await?;
        Ok(())
    }

    /// Fetches source `id` after the call that asked for it, unless a fetch
    /// of it runs already; the call's port answers for as long as the fetch
    /// runs. Answers the end of the fetch that runs.
    fn start_fetch(self: &Rc<Self>, port: PluginPort, id: String) -> FetchEnd {
        if let Some(running) = self.fetching.borrow().get(&id) {
            return running.clone();
        }
        let (end, ended) = watch::channel(None);
        self.fetching.borrow_mut().insert(id.clone(), ended.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let state = self.clone();
        let fetch_stop = stop.clone();
        let task = tokio::task::spawn_local(async move {
            let fetched = state.fetch_source(&port, &id, fetch_stop).await;
            if let Err(error) = &fetched {
                tracing::warn!(event = "skill_source.fetch_record_failed", source = %id, %error, "a skill source's fetch was not recorded");
            }
            state.fetching.borrow_mut().remove(&id);
            if matches!(fetched, Ok(Fetched::Superseded)) {
                // The fetch of the source's new fetcher, which those waiting
                // for this one hear the end of instead.
                let mut next = state.start_fetch(port.clone(), id.clone());
                let next = next.wait_for(Option::is_some).await.map(|ended| ended.clone()).ok().flatten();
                end.send_replace(next.or(Some(Ok(Fetched::Superseded))));
                return;
            }
            end.send_replace(Some(fetched.map_err(|error| error.to_string())));
            // A page that misses this change reads the state again when it
            // reconnects.
            if let Err(error) = port.changed(Scope::User).await {
                tracing::warn!(event = "skill_source.fetch_announce_failed", source = %id, %error, "the pages did not learn of a fetch's end");
            }
        });
        let mut fetches = self.fetches.borrow_mut();
        fetches.retain(|(task, _)| !task.is_finished());
        fetches.push((task.abort_handle(), stop));
        ended
    }

    async fn fetch_source(
        &self,
        port: &PluginPort,
        id: &str,
        stop: Arc<AtomicBool>,
    ) -> Result<Fetched, PluginError> {
        port.changed(Scope::User).await?;
        let (source, _) = sources::read_one(port, id).await?;
        let origin = Origin::parse(&source.origin)
            .map_err(|message| PluginError::failed(format!("a stored origin: {message}")))?;
        // What fetches it now, which also decides whose heads learn the
        // commit: a device's commit never reaches the shared heads.
        let fetcher = source.fetcher.clone();
        let fetched = match &fetcher {
            None => {
                let url = (self.resolve)(origin.url());
                let repository = origin.repository().to_owned();
                tokio::task::spawn_blocking(move || fetch::fetch(&url, &repository, &stop))
                    .await
                    .map_err(|error| PluginError::failed(format!("the fetch ended: {error}")))?
            }
            Some(device) => fetch_on_device(port, device, &origin).await,
        };
        let outcome = match fetched {
            Ok(fetched) => {
                let blobs = sources::put_files(port, &fetched).await?;
                Ok((fetched, blobs))
            }
            Err(FetchError::Stopped) => return Ok(Fetched::Stopped),
            Err(FetchError::Failed { message, problem }) => Err((message, problem)),
        };
        loop {
            let (all, _) = self.sources(port).await?;
            let Some((source, revision)) = all.get(id) else {
                return Ok(Fetched::Removed);
            };
            if source.fetcher != fetcher {
                return Ok(Fetched::Superseded);
            }
            let now = self.clock.now();
            let recorded = match &outcome {
                Ok((fetched, blobs)) => sources::fetched(&all, id, fetched, blobs, now),
                Err((message, problem)) => Source {
                    failure: Some(Failure {
                        at: now,
                        message: message.clone(),
                        problem: Some(problem.clone()),
                    }),
                    ..source.clone()
                },
            };
            match sources::write(port, id, &recorded, Some(*revision)).await {
                Ok(_) => {
                    if let Ok((fetched, _)) = &outcome {
                        let heads = if fetcher.is_some() { &self.device_heads } else { &*self.heads };
                        heads.fetched(id, now, fetched.commit.clone());
                    }
                    break;
                }
                Err(failure) if sources::conflict(&failure) => continue,
                Err(failure) => return Err(failure.into()),
            }
        }
        match outcome {
            Ok(_) => {
                let (all, _) = self.sources(port).await?;
                port.set_directories(sources::directories(&all)).await?;
                Ok(Fetched::Pinned)
            }
            Err((message, _)) => Ok(Fetched::Failed(message)),
        }
    }

    /// Starts the task that follows the user's sources, unless it runs:
    /// every [`CHECK_INTERVAL`] it checks the default branch of each source
    /// due, and it tells the pages, through `port`, of each move of one of
    /// the user's sources, whoever's check or fetch found it.
    fn follow(self: &Rc<Self>, port: &PluginPort) {
        let mut follow = self.follow.borrow_mut();
        if follow.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        // Subscribed before the state is read, so a move published meanwhile
        // is told, if once more than needed.
        let mut moves = self.heads.moves();
        let state = self.clone();
        let port = port.clone();
        let task = tokio::task::spawn_local(async move {
            let mut ticks = tokio::time::interval(CHECK_INTERVAL);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = ticks.tick() => {
                        if let Err(error) = state.check_due(&port).await {
                            tracing::warn!(event = "skill_sources.check_failed", %error, "the skill sources were not checked");
                        }
                    }
                    moved = moves.recv() => {
                        let ours = match moved {
                            Ok(id) => sources::read(&port).await.map(|all| all.contains_key(&id)),
                            // A move missed may be one of the user's.
                            Err(broadcast::error::RecvError::Lagged(_)) => Ok(true),
                            Err(broadcast::error::RecvError::Closed) => return,
                        };
                        // A page that misses this change reads the state
                        // again when it reconnects.
                        let told = match ours {
                            Ok(true) => port.changed(Scope::User).await.map_err(PluginError::from),
                            Ok(false) => Ok(()),
                            Err(error) => Err(error),
                        };
                        if let Err(error) = told {
                            tracing::warn!(event = "skill_source.move_announce_failed", %error, "the pages did not learn of a source's move");
                        }
                    }
                }
            }
        });
        *follow = Some(task.abort_handle());
    }

    /// Checks the default branch of each of the user's sources that has a
    /// commit, is not being fetched, and was checked by no user within
    /// [`CHECK_INTERVAL`]; a move found reaches every user's pages through
    /// the heads.
    async fn check_due(&self, port: &PluginPort) -> Result<(), PluginError> {
        let (all, _) = self.sources(port).await?;
        let now = self.clock.now();
        let mut due: Vec<(String, String)> = Vec::new();
        let mut on_devices: Vec<(String, DeviceId, String)> = Vec::new();
        let online: Vec<DeviceId> = if all.values().any(|(source, _)| source.fetcher.is_some()) {
            port.devices().await?.into_iter().filter(|device| device.online).map(|device| device.id).collect()
        } else {
            Vec::new()
        };
        for (id, (source, _)) in all {
            if source.commit.is_none() || self.fetching.borrow().contains_key(&id) {
                continue;
            }
            if let Some(device) = &source.fetcher {
                // An offline device checks nothing; the source waits for it.
                if online.contains(device) && self.device_heads.claim(&id, now) {
                    let origin = Origin::parse(&source.origin)
                        .map_err(|message| PluginError::failed(format!("a stored origin: {message}")))?;
                    on_devices.push((id, device.clone(), origin.url().to_owned()));
                }
                continue;
            }
            let origin = Origin::parse(&source.origin)
                .map_err(|message| PluginError::failed(format!("a stored origin: {message}")))?;
            if self.heads.claim(&id, now) {
                due.push((id, (self.resolve)(origin.url())));
            }
        }
        let checks = due.into_iter().map(|(id, url)| async move {
            let started = std::time::Instant::now();
            let checked = tokio::task::spawn_blocking(move || fetch::remote_head(&url)).await;
            (id, checked, u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX))
        });
        // Each device's check has its own time limit, and none waits for
        // another or for the server's.
        let device_checks = on_devices.into_iter().map(|(id, device, url)| async move {
            let checked = port.device_git_head(device, url).await;
            (id, checked)
        });
        let (device_checked, server_checked) = futures_util::future::join(join_all(device_checks), join_all(checks)).await;
        let mut moved = false;
        for (id, checked) in device_checked {
            let before = self.device_heads.of([&id]).remove(&id).and_then(|head| head.commit);
            match checked {
                Ok(commit) => {
                    moved |= before.as_ref() != Some(&commit);
                    self.device_heads.checked(&id, now, commit);
                }
                Err(error) => {
                    tracing::info!(event = "skill_source.branch_unchecked", source = %id, reason = %error, "a skill source's default branch was not checked on its device");
                }
            }
        }
        if moved {
            port.changed(Scope::User).await?;
        }
        for (id, checked, ms) in server_checked {
            match checked {
                Ok(Ok(commit)) => self.heads.checked(&id, now, commit),
                Ok(Err(message)) => {
                    tracing::info!(event = "skill_source.branch_unchecked", source = %id, reason = %message, ms, "a skill source's default branch was not checked");
                }
                Err(error) => {
                    tracing::warn!(event = "skill_source.check_ended", source = %id, %error, ms, "a skill source's check ended without an answer");
                }
            }
        }
        Ok(())
    }

    /// The node's catalog block, when it differs from the newest of `seen`,
    /// the plugin's blocks the model receives: the project skills the
    /// backend holds, `project`, and the user's skills that are on in Global
    /// and in the conversation's workspace.
    async fn context(
        &self,
        port: &PluginPort,
        project: Option<Value>,
        seen: &[String],
    ) -> Result<Option<String>, PluginError> {
        let project: Vec<ProjectSkill> = match project.map(serde_json::from_value) {
            None => Vec::new(),
            Some(Ok(skills)) => skills,
            // What the backend holds is what this plugin's search answered.
            Some(Err(error)) => {
                return Err(PluginError::failed(format!("the project skills held do not read: {error}")));
            }
        };
        let mut entries: Vec<Entry> = Vec::new();
        for skill in project {
            if !skill.disable_model_invocation {
                entries.push(Entry {
                    name: skill.name,
                    description: skill.description,
                    location: skill.location,
                });
            }
        }
        let (all, places) = self.sources(port).await?;
        for (source, _) in all.into_values() {
            if !places.offered(source.workspace.as_ref()) {
                continue;
            }
            for skill in source.skills.iter().filter(|skill| skill.enabled) {
                let shadowed = entries.iter().any(|entry| entry.name == skill.name);
                if shadowed {
                    tracing::info!(event = "skill.shadowed", skill = %skill.name, origin = %source.origin, by = "project", "a user skill is shadowed by a project skill");
                    continue;
                }
                if !skill.disable_model_invocation {
                    entries.push(Entry {
                        name: skill.name.clone(),
                        description: skill.description.clone(),
                        location: sources::location(&self.plugin, skill),
                    });
                }
            }
        }
        let text = if entries.is_empty() {
            (!seen.is_empty()).then(|| NONE_AVAILABLE.to_owned())
        } else {
            Some(catalog::render(&entries))
        };
        Ok(text.filter(|text| seen.last() != Some(text)))
    }
}

fn decode<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, PluginError> {
    serde_json::from_value(Value::Object(params)).map_err(|error| PluginError::Usage {
        message: error.to_string(),
    })
}

/// Fetches `origin` on `device` with its own `git`, as [`fetch::fetch`] does
/// on the server.
async fn fetch_on_device(port: &PluginPort, device: &DeviceId, origin: &Origin) -> Result<fetch::Fetched, FetchError> {
    let tree = port
        .device_git_fetch(
            device.clone(),
            origin.url().to_owned(),
            "SKILL.md".to_owned(),
            fetch::FETCH_MAX_BYTES as u64,
            fetch::FILES_MAX_BYTES as u64,
        )
        .await
        .map_err(|failure| match failure {
            PortFailure::Refused(PortRefusal::Git { refusal }) => match refusal {
                GitRefusal::RepositoryTooLarge { limit } => FetchError::Failed {
                    message: format!("the repository is larger than {} MiB", limit / (1024 * 1024)),
                    problem: fetch::FetchProblem::RepositoryTooLarge { limit_bytes: limit as u32 },
                },
                GitRefusal::FilesTooLarge { limit } => FetchError::Failed {
                    message: format!("the skills' files hold more than {} MiB", limit / (1024 * 1024)),
                    problem: fetch::FetchProblem::FilesTooLarge { limit_bytes: limit as u32 },
                },
                // The device went away: nothing is recorded, and the source
                // waits for it as for an offline device.
                GitRefusal::Offline => FetchError::Stopped,
                refusal => FetchError::Failed { message: refusal.to_string(), problem: fetch::FetchProblem::Failed },
            },
            failure => FetchError::Failed { message: failure.to_string(), problem: fetch::FetchProblem::Failed },
        })?;
    let files = tree
        .entries
        .into_iter()
        .map(|entry| fetch::TreeFile { path: entry.path, executable: entry.executable })
        .collect();
    let bytes = tree.files.into_iter().map(|file| (file.path, file.bytes.into_bytes().to_vec())).collect();
    fetch::from_device(tree.commit, files, bytes, origin.repository())
}
