//! What the shard keeps of its conversations' project files
//! (`crates/backend-user-shard/src/conversation/workflows.rs`).
//!
//! The shard keeps three things, per Host and directory:
//!
//! - the instruction files of each node's working directory;
//! - the workflows of each conversation's directory;
//! - each plugin's project search of it.
//!
//! The runner's file watch keeps them current through host access ([`Kept`]). An
//! agent's request takes what is held; only a directory's first search holds
//! it, for at most 5 seconds. The workflows a page shows are stored as each
//! search finds them (`host_workflows`), and the page reads what is stored at
//! once.

use std::rc::{Rc, Weak};

use demi_backend_database::conversation_index::ExecutionTarget;
use demi_backend_host_access::access::HostAccessError;
use demi_backend_host_access::conversation_of;
use demi_backend_host_access::kept::{Found, Kept, SearchError, Shown, Subscribe};
use demi_backend_page_sync::Part;
use demi_plugin_interface::{PluginId, WatchSubject};
use demi_runner_protocol::files::Subject;
use demi_shared_types::NodeId;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use demi_web_api_protocol::workflows::{ConversationWorkflowList, KeptWorkflowList};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::instructions::{ProjectFile, search_instructions};
use super::workflows::{ProjectWorkflows, search_conversation_workflows};
use crate::shard::Shard;

pub(crate) struct ProjectFiles {
    /// The instruction files of each working directory.
    pub(crate) instructions: Kept<String, Rc<[ProjectFile]>>,
    /// The workflows of each conversation directory.
    pub(crate) workflows: Kept<String, Rc<ProjectWorkflows>>,
    /// What each plugin's project search found in each conversation
    /// directory.
    pub(crate) plugins: Kept<(PluginId, String), Value>,
}

impl ProjectFiles {
    pub(crate) fn new(shard: &Weak<Shard>) -> Self {
        let instructions = {
            let searcher = shard.clone();
            Kept::new(
                "instruction files",
                Box::new(move |conversation: ConversationId, device: DeviceId, cwd: String| {
                    let shard = searcher.clone();
                    Box::pin(async move {
                        let shard = shard.upgrade().ok_or_else(closing)?;
                        let found = search_instructions(&shard, &conversation, &device, &cwd, &CancellationToken::new()).await?;
                        Ok(found)
                    })
                }),
                subscribe(shard.clone()),
                None,
            )
        };
        let workflows = {
            let searcher = shard.clone();
            Kept::new(
                "workflows",
                Box::new(move |conversation: ConversationId, device: DeviceId, directory: String| {
                    let shard = searcher.clone();
                    Box::pin(async move {
                        let shard = shard.upgrade().ok_or_else(closing)?;
                        let found = search_conversation_workflows(&shard, &conversation, &device, &directory).await?;
                        Ok(Found {
                            watch: subjects(&found.watch),
                            value: Rc::new(found),
                        })
                    })
                }),
                subscribe(shard.clone()),
                Some(shown_workflows(shard.clone())),
            )
        };
        let plugins = {
            let searcher = shard.clone();
            Kept::new(
                "a plugin's project search",
                Box::new(move |conversation, device: DeviceId, (plugin, directory): (PluginId, String)| {
                    let shard = searcher.clone();
                    Box::pin(async move {
                        let shard = shard.upgrade().ok_or_else(closing)?;
                        let paths = shard
                            .host_paths(&conversation)
                            .await
                            .map_err(|error| SearchError::Failed(error.to_string()))?;
                        let found = shard
                            .plugins()
                            .project_search(&plugin, &conversation, &device, directory, paths, CancellationToken::new())
                            .await
                            .map_err(|error| match error {
                                demi_plugin_interface::PluginError::Port {
                                    refusal: demi_plugin_interface::PortRefusal::NotRunning,
                                } => SearchError::NotRunning,
                                error => SearchError::Failed(format!("{plugin}: {error}")),
                            })?;
                        // Turned off meanwhile: its switch forgets the entry.
                        let Some((found, watch)) = found else {
                            return Err(SearchError::Failed(format!("{plugin} is off")));
                        };
                        Ok(Found {
                            value: found,
                            watch: subjects(&watch),
                        })
                    })
                }),
                subscribe(shard.clone()),
                None,
            )
        };
        Self {
            instructions,
            workflows,
            plugins,
        }
    }

    /// The Host of `device` came online again.
    pub(crate) fn online(&self, device: &DeviceId) {
        self.instructions.online(device);
        self.workflows.online(device);
        self.plugins.online(device);
    }

    /// The conversation's tree closed: it no longer keeps anything.
    pub(crate) fn release(&self, conversation: &ConversationId) {
        self.instructions.release(conversation);
        self.workflows.release(conversation);
        self.plugins.release(conversation);
    }
}

/// Where a conversation's project files are kept.
pub(crate) struct KeptAt {
    /// Its primary Host's device; none for a Cloud not made yet.
    device: Option<DeviceId>,
    /// The directory it works in there.
    directory: String,
    /// Whether it is in a project.
    in_project: bool,
    /// Whether its tree is loaded.
    loaded: bool,
}

impl Shard {
    /// Where the conversation `id`'s project files are kept, read without
    /// reaching its Host.
    async fn kept_at(&self, id: &ConversationId) -> Result<KeptAt, HostAccessError> {
        let host = self.host_shard();
        let record = host.owned_conversation(id).await?;
        let target = host.resolve_target(&record).await?;
        Ok(KeptAt {
            device: target.device().cloned(),
            directory: target.path().to_owned(),
            in_project: matches!(target, ExecutionTarget::Workspace { .. }),
            loaded: self.agent().tree(&demi_backend_host_access::root_of(id)).is_some(),
        })
    }

    /// The instruction files of `cwd` for the conversation `id`.
    pub(crate) async fn kept_instructions(&self, id: &ConversationId, cwd: &str) -> Rc<[ProjectFile]> {
        let at = match self.kept_at(id).await {
            Ok(at) => at,
            Err(error) => {
                tracing::warn!(conversation = %id, %error, "the project's instruction files were not taken");
                return Rc::from([]);
            }
        };
        let Some(device) = at.device else {
            return Rc::from([]);
        };
        self.project_files()
            .instructions
            .get(id, &device, cwd.to_owned(), at.loaded)
            .await
            // A refusal is logged where the search met it.
            .ok()
            .flatten()
            .unwrap_or_else(|| Rc::from([]))
    }

    /// The workflows of the conversation `id`'s directory, or that no search
    /// of it is held, such as on a Host that never ran; the refusal when the
    /// last search was refused or failed and nothing is held.
    pub(crate) async fn kept_workflows(&self, id: &ConversationId) -> Result<KeptWorkflows, Rc<HostAccessError>> {
        let at = self.kept_at(id).await.map_err(Rc::new)?;
        let Some(device) = at.device else {
            return Ok(KeptWorkflows::None);
        };
        let found = self
            .project_files()
            .workflows
            .get(id, &device, at.directory.clone(), at.loaded)
            .await?;
        Ok(match found {
            Some(found) => KeptWorkflows::Found(found),
            None => KeptWorkflows::None,
        })
    }

    /// What `plugin`'s project search holds for the conversation of `root`.
    ///
    /// None outside a project, whatever its directory, which is then neither
    /// searched nor watched: a conversation working in the home folder would
    /// otherwise take another agent's own skills for a project's. A move into or
    /// out of a project releases what was kept, so the next request starts or
    /// skips the search.
    pub(crate) async fn kept_project_search(&self, plugin: &PluginId, root: &NodeId) -> Option<Value> {
        let id = conversation_of(root);
        let at = match self.kept_at(&id).await {
            Ok(at) => at,
            Err(error) => {
                tracing::warn!(conversation = %id, %plugin, %error, "a plugin's project search was not taken");
                return None;
            }
        };
        if !at.in_project {
            return None;
        }
        let key = (plugin.clone(), at.directory);
        self.project_files()
            .plugins
            .get(&id, &at.device?, key, at.loaded)
            .await
            // A refusal is logged where the search met it.
            .ok()
            .flatten()
    }
}

impl Shard {
    /// The Workflow menu's list for the conversation `id` as its metadata
    /// carries it, as the backend stored it, at once, under its revision:
    /// unknown before its first read; the refusal when the last search was
    /// refused or failed and nothing was ever stored. A loaded conversation
    /// keeps it current, which announces each change in its metadata.
    pub(crate) async fn conversation_workflows(&self, id: &ConversationId) -> ConversationWorkflowList {
        let refused = |error: &HostAccessError| {
            let (code, status) = error.code();
            ConversationWorkflowList {
                revision: 0,
                list: KeptWorkflowList::Refused {
                    code,
                    status,
                    message: error.to_string(),
                },
            }
        };
        let at = match self.kept_at(id).await {
            Ok(at) => at,
            Err(error) => return refused(&error),
        };
        let unknown = ConversationWorkflowList {
            revision: 0,
            list: KeptWorkflowList::Unknown,
        };
        let Some(device) = at.device else {
            return unknown;
        };
        let kept = &self.project_files().workflows;
        if at.loaded {
            kept.keep(id, &device, at.directory.clone());
        }
        match self.services().control.host_workflows(device.clone(), at.directory.clone()).await {
            Ok(Some(stored)) => ConversationWorkflowList {
                revision: stored.revision,
                list: KeptWorkflowList::Read {
                    list: stored.list,
                    read_at: stored.read_at,
                },
            },
            Ok(None) => match kept.refused(&device, at.directory) {
                Some(error) => refused(&*error),
                None => unknown,
            },
            Err(error) => refused(&HostAccessError::from(error)),
        }
    }
}

/// Stores each search of a directory's workflows in the menu's form, and
/// announces a changed one in the metadata of the conversations that use it.
fn shown_workflows(shard: Weak<Shard>) -> Shown<String, Rc<ProjectWorkflows>> {
    let announcer = shard.clone();
    Shown {
        store: Box::new(move |device: DeviceId, directory: String, found: &Rc<ProjectWorkflows>| {
            let shard = shard.clone();
            let list = found.menu();
            Box::pin(async move {
                let shard = shard.upgrade().ok_or("the backend is shutting down")?;
                shard
                    .services()
                    .control
                    .keep_host_workflows(device, directory, list)
                    .await
                    .map_err(|error| error.to_string())
            })
        }),
        announce: Box::new(move |conversations| {
            if let Some(shard) = announcer.upgrade() {
                for conversation in conversations {
                    shard.mark(Part::Metadata(conversation.clone()));
                }
            }
        }),
    }
}

/// What the shard holds of a conversation's workflows.
pub(crate) enum KeptWorkflows {
    /// What the last search found.
    Found(Rc<ProjectWorkflows>),
    /// No search of the conversation's working directory is held: its Host
    /// is not running, or the first search has not finished.
    None,
}

/// What a search names to watch, as the runner subscribes to it.
fn subjects(watch: &[WatchSubject]) -> Vec<Subject> {
    watch
        .iter()
        .map(|subject| Subject {
            kind: subject.kind,
            path: subject.path.clone(),
            suffix: subject.suffix.clone(),
        })
        .collect()
}

/// Subscriptions on a conversation's primary Host, through its host access.
fn subscribe(shard: Weak<Shard>) -> Subscribe {
    Box::new(move |conversation| {
        let shard = shard.clone();
        Box::pin(async move {
            let shard = shard.upgrade().ok_or_else(closing)?;
            let watching = shard
                .host_shard()
                .watch_files(&conversation, &CancellationToken::new())
                .await?;
            Ok(watching)
        })
    })
}

fn closing() -> SearchError {
    SearchError::Failed("the backend is shutting down".to_owned())
}
