//! The `skills` plugin (`skills.md`): the user's skill sources, fetched
//! from git and pinned to a commit, whose skills the user turns on one by
//! one and which every Host the user's jobs run on receives as a directory;
//! the skills a repository carries; and the catalog of both, which reaches
//! the model as a context block. Nothing else in Demi knows skills.

mod catalog;
mod fetch;
mod origin;
mod project;
mod skill;
mod sources;
#[cfg(feature = "testing")]
pub mod testing;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use demi_plugin_interface::{
    Manifest, Method, Page, Plugin, PluginError, PluginId, PluginPort, PortFailure, PortRefusal,
    Reply, Request, Scope,
};
use demi_shared_types::{Clock, SystemClock, TurnId};
use futures_util::future::{LocalBoxFuture, join_all};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use tokio::task::AbortHandle;

use crate::catalog::{Entry, NONE_AVAILABLE};
use crate::fetch::FetchError;
use crate::origin::Origin;
use crate::project::ProjectSkill;
use crate::sources::{Head, Source};

pub use crate::fetch::Skipped;
pub use crate::sources::{Failure, SkillState, SkillsState, SourceState};

/// How long a source's check of its default branch counts before the page's
/// next opening checks it again (`skills.md` § Updates available).
const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Maps a source's URL to the URL its fetch reads: the URL itself, or in a
/// test, a repository the test made.
pub type Resolve = dyn Fn(&str) -> String + Send + Sync;

/// The plugin's factory.
pub struct Skills {
    manifest: Manifest,
    resolve: Arc<Resolve>,
    clock: Arc<dyn Clock>,
}

impl Skills {
    pub fn new() -> Self {
        Self::resolving(Arc::new(str::to_owned), Arc::new(SystemClock))
    }

    /// A factory whose fetches read `resolve`'s URL for each source's, with
    /// `clock` telling when a fetch ended: a test's repositories and time.
    pub fn resolving(resolve: Arc<Resolve>, clock: Arc<dyn Clock>) -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("skills").expect("a valid plugin id"),
            "Skills",
            "Workflows the agent follows: skills from Git repositories you add, and those your repository carries.",
        );
        manifest.context = true;
        manifest.page = Some(page());
        Self {
            manifest,
            resolve,
            clock,
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
            plugin: self.manifest.id.clone(),
            resolve: self.resolve.clone(),
            clock: self.clock.clone(),
            fetching: RefCell::default(),
            fetches: RefCell::default(),
            heads: RefCell::default(),
            checks: RefCell::default(),
            projects: RefCell::default(),
        })))
    }
}

/// `add_source { origin }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddSource {
    pub origin: String,
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

/// `check_updates {}`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckUpdates {}

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
        .method(Method::new::<SourceCall, ()>("remove_source", Scope::User))
        .method(Method::new::<SetEnabled, ()>("set_enabled", Scope::User))
        .method(Method::new::<SetSourceEnabled, ()>(
            "set_source_enabled",
            Scope::User,
        ))
        .method(Method::new::<CheckUpdates, ()>("check_updates", Scope::User))
}

/// One user's skills plugin.
struct Instance(Rc<State>);

/// What the instance keeps in memory, for its user only.
struct State {
    plugin: PluginId,
    resolve: Arc<Resolve>,
    clock: Arc<dyn Clock>,
    /// The sources being fetched, by id.
    fetching: RefCell<BTreeSet<String>>,
    /// Each running fetch: its task and what stops its blocking part.
    fetches: RefCell<Vec<(AbortHandle, Arc<AtomicBool>)>>,
    /// What each source's default branch points to, by id.
    heads: RefCell<BTreeMap<String, Head>>,
    /// Each running check of the sources' default branches.
    checks: RefCell<Vec<AbortHandle>>,
    /// The last project search's skills, by conversation and working
    /// directory, with the input turn it succeeded at.
    projects: RefCell<HashMap<(String, String), ProjectSearch>>,
}

struct ProjectSearch {
    turn: TurnId,
    skills: Vec<ProjectSkill>,
}

/// The instance ends with its shard: every fetch stops and saves nothing.
impl Drop for Instance {
    fn drop(&mut self) {
        for (task, stop) in self.0.fetches.take() {
            stop.store(true, Ordering::Relaxed);
            task.abort();
        }
        // A check's blocking part only reads the remote's refs; it ends on
        // its own, and its result goes nowhere.
        for task in self.0.checks.take() {
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
                Request::Context {
                    conversation,
                    cwd,
                    turn,
                    seen,
                    ..
                } => {
                    let asked = Asked {
                        conversation: conversation.to_string(),
                        cwd,
                        turn,
                        seen,
                    };
                    let text = self.0.context(&port, asked).await?;
                    Ok(Reply::Context { text })
                }
                Request::PageState { .. } => {
                    let sources = sources::read(&port).await?;
                    Ok(Reply::State {
                        state: sources::state(
                            &sources,
                            &self.0.fetching.borrow(),
                            &self.0.heads.borrow(),
                        ),
                    })
                }
                Request::PageCall { method, params, .. } => Ok(Reply::Result {
                    result: self.0.call(&method, params, port).await?,
                }),
                Request::Command { .. } => Err(PluginError::undeclared("command")),
                Request::PanelTab { .. } => Err(PluginError::undeclared("panel kind")),
                Request::Topic { .. } => Err(PluginError::undeclared("topic")),
            }
        })
    }
}

/// A node's context request; the catalog is the same for every node of a
/// working directory.
struct Asked {
    conversation: String,
    cwd: String,
    turn: TurnId,
    seen: Vec<String>,
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
                let AddSource { origin } = decode(params)?;
                let source = self.add_source(&port, &origin).await?;
                self.start_fetch(port, source.clone());
                Ok(serde_json::to_value(AddedSource { source }).expect("a result encodes"))
            }
            "update_source" => {
                let SourceCall { source } = decode(params)?;
                sources::read_one(&port, &source).await?;
                if !self.fetching.borrow().contains(&source) {
                    self.start_fetch(port, source);
                }
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
            "check_updates" => {
                let CheckUpdates {} = decode(params)?;
                self.start_checks(port).await?;
                Ok(Value::Null)
            }
            method => Err(PluginError::failed(format!("no method \"{method}\""))),
        }
    }

    /// Records the source `origin` names, with no commit and no skills, and
    /// answers its id.
    async fn add_source(&self, port: &PluginPort, origin: &str) -> Result<String, PluginError> {
        let parsed = Origin::parse(origin)
            .map_err(|message| PluginError::refused("invalid_origin", message))?;
        let id = parsed.id();
        let existing = sources::read(port).await?;
        let taken = || {
            PluginError::refused(
                "source_exists",
                format!("{} is added already", parsed.url()),
            )
        };
        if existing.contains_key(&id) {
            return Err(taken());
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
        };
        match sources::write(port, &id, &source, None).await {
            Ok(_) => {}
            Err(failure) if sources::conflict(&failure) => return Err(taken()),
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
        self.heads.borrow_mut().remove(id);
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
            let all = sources::read(port).await?;
            let (source, revision) = all.get(id).ok_or_else(|| sources::not_found(id))?;
            let changed = sources::switch(&all, id, &chosen(source), enabled)?;
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
        let all = sources::read(port).await?;
        port.set_directories(sources::directories(&all)).await?;
        port.changed(Scope::User).await?;
        Ok(())
    }

    /// Fetches source `id` after the call that asked for it; the call's
    /// port answers for as long as the fetch runs. One fetch of a source
    /// runs at a time.
    fn start_fetch(self: &Rc<Self>, port: PluginPort, id: String) {
        self.fetching.borrow_mut().insert(id.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let state = self.clone();
        let fetch_stop = stop.clone();
        let task = tokio::task::spawn_local(async move {
            if let Err(error) = state.fetch_source(&port, &id, fetch_stop).await {
                tracing::warn!(source = %id, %error, "a skill source's fetch was not recorded");
            }
            state.fetching.borrow_mut().remove(&id);
            // A page that misses this change reads the state again when it
            // reconnects.
            if let Err(error) = port.changed(Scope::User).await {
                tracing::warn!(source = %id, %error, "the pages did not learn of a fetch's end");
            }
        });
        let mut fetches = self.fetches.borrow_mut();
        fetches.retain(|(task, _)| !task.is_finished());
        fetches.push((task.abort_handle(), stop));
    }

    async fn fetch_source(
        &self,
        port: &PluginPort,
        id: &str,
        stop: Arc<AtomicBool>,
    ) -> Result<(), PluginError> {
        port.changed(Scope::User).await?;
        let (source, _) = sources::read_one(port, id).await?;
        let origin = Origin::parse(&source.origin)
            .map_err(|message| PluginError::failed(format!("a stored origin: {message}")))?;
        let url = (self.resolve)(origin.url());
        let repository = origin.repository().to_owned();
        let fetched = tokio::task::spawn_blocking(move || fetch::fetch(&url, &repository, &stop))
            .await
            .map_err(|error| PluginError::failed(format!("the fetch ended: {error}")))?;
        let outcome = match fetched {
            Ok(fetched) => {
                let blobs = sources::put_files(port, &fetched).await?;
                Ok((fetched, blobs))
            }
            Err(FetchError::Stopped) => return Ok(()),
            Err(FetchError::Failed(message)) => Err(message),
        };
        loop {
            let all = sources::read(port).await?;
            let Some((source, revision)) = all.get(id) else {
                // Removed while it was fetched.
                return Ok(());
            };
            let now = self.clock.now();
            let recorded = match &outcome {
                Ok((fetched, blobs)) => sources::fetched(&all, id, fetched, blobs, now),
                Err(message) => Source {
                    failure: Some(Failure {
                        at: now,
                        message: message.clone(),
                    }),
                    ..source.clone()
                },
            };
            match sources::write(port, id, &recorded, Some(*revision)).await {
                Ok(_) => {
                    if let Ok((fetched, _)) = &outcome {
                        let head = Head {
                            checked_at: now,
                            commit: Some(fetched.commit.clone()),
                        };
                        self.heads.borrow_mut().insert(id.to_owned(), head);
                    }
                    break;
                }
                Err(failure) if sources::conflict(&failure) => continue,
                Err(failure) => return Err(failure.into()),
            }
        }
        if outcome.is_ok() {
            let all = sources::read(port).await?;
            port.set_directories(sources::directories(&all)).await?;
        }
        Ok(())
    }

    /// Checks, after the call that asked for it, the default branch of each
    /// source that has a commit, is not being fetched, and was not checked
    /// within [`CHECK_INTERVAL`]; the pages receive the new state once every
    /// check ended, if one found another commit.
    async fn start_checks(self: &Rc<Self>, port: PluginPort) -> Result<(), PluginError> {
        let all = sources::read(&port).await?;
        let now = self.clock.now();
        let interval = i64::try_from(CHECK_INTERVAL.as_millis()).expect("minutes fit");
        let mut due: Vec<(String, String)> = Vec::new();
        {
            let fetching = self.fetching.borrow();
            let mut heads = self.heads.borrow_mut();
            for (id, (source, _)) in all {
                let recent = heads.get(&id).is_some_and(|head| {
                    now.as_millisecond() - head.checked_at.as_millisecond() < interval
                });
                if source.commit.is_none() || fetching.contains(&id) || recent {
                    continue;
                }
                let origin = Origin::parse(&source.origin)
                    .map_err(|message| PluginError::failed(format!("a stored origin: {message}")))?;
                let commit = heads.get(&id).and_then(|head| head.commit.clone());
                let head = Head {
                    checked_at: now,
                    commit,
                };
                heads.insert(id.clone(), head);
                due.push((id, (self.resolve)(origin.url())));
            }
        }
        if due.is_empty() {
            return Ok(());
        }
        let state = self.clone();
        let task = tokio::task::spawn_local(async move {
            let checks = due.into_iter().map(|(id, url)| async move {
                let checked = tokio::task::spawn_blocking(move || fetch::remote_head(&url)).await;
                (id, checked)
            });
            let mut changed = false;
            for (id, checked) in join_all(checks).await {
                let commit = match checked {
                    Ok(Ok(commit)) => commit,
                    Ok(Err(message)) => {
                        tracing::info!(source = %id, %message, "a skill source's default branch was not checked");
                        continue;
                    }
                    Err(error) => {
                        tracing::warn!(source = %id, %error, "a skill source's check ended");
                        continue;
                    }
                };
                let mut heads = state.heads.borrow_mut();
                // A fetch that ended since, or a removal, knows better.
                let Some(head) = heads.get_mut(&id).filter(|head| head.checked_at == now) else {
                    continue;
                };
                if head.commit.as_ref() != Some(&commit) {
                    head.commit = Some(commit);
                    changed = true;
                }
            }
            // A page that misses this change reads the state again when it
            // reconnects.
            if changed && let Err(error) = port.changed(Scope::User).await {
                tracing::warn!(%error, "the pages did not learn of a check's end");
            }
        });
        let mut checks = self.checks.borrow_mut();
        checks.retain(|task| !task.is_finished());
        checks.push(task.abort_handle());
        Ok(())
    }

    /// The node's catalog block, when it differs from the newest the model
    /// receives.
    async fn context(
        &self,
        port: &PluginPort,
        asked: Asked,
    ) -> Result<Option<String>, PluginError> {
        let mut entries: Vec<Entry> = Vec::new();
        for skill in self.project_skills(port, &asked).await {
            if !skill.disable_model_invocation {
                entries.push(Entry {
                    name: skill.name,
                    description: skill.description,
                    location: skill.location,
                });
            }
        }
        for (source, _) in sources::read(port).await?.into_values() {
            for skill in source.skills.iter().filter(|skill| skill.enabled) {
                let shadowed = entries.iter().any(|entry| entry.name == skill.name);
                if shadowed {
                    tracing::info!(skill = %skill.name, origin = %source.origin, "a user skill is shadowed by a project skill");
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
            (!asked.seen.is_empty()).then(|| NONE_AVAILABLE.to_owned())
        } else {
            Some(catalog::render(&entries))
        };
        Ok(text.filter(|text| asked.seen.last() != Some(text)))
    }

    /// The project skills for the node: searched at the first request of
    /// each input turn, and again at the next request of the turn while the
    /// Host was not running; what the last search found otherwise.
    async fn project_skills(&self, port: &PluginPort, asked: &Asked) -> Vec<ProjectSkill> {
        let key = (asked.conversation.clone(), asked.cwd.clone());
        if let Some(search) = self.projects.borrow().get(&key)
            && search.turn == asked.turn
        {
            return search.skills.clone();
        }
        match project::search(port, &asked.cwd).await {
            Ok(skills) => {
                let search = ProjectSearch {
                    turn: asked.turn.clone(),
                    skills: skills.clone(),
                };
                self.projects.borrow_mut().insert(key, search);
                skills
            }
            Err(failure) => {
                if !matches!(failure, PortFailure::Refused(PortRefusal::NotRunning)) {
                    tracing::warn!(cwd = %asked.cwd, %failure, "the project skills were not searched");
                }
                self.projects
                    .borrow()
                    .get(&key)
                    .map(|search| search.skills.clone())
                    .unwrap_or_default()
            }
        }
    }
}

fn decode<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, PluginError> {
    serde_json::from_value(Value::Object(params)).map_err(|error| PluginError::Usage {
        message: error.to_string(),
    })
}
