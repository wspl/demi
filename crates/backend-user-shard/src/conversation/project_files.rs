//! What the shard keeps of its conversations' project files
//! (`instructions.md` § When it is sent, `skills.md` § Project skills,
//! `subagents.md` § Workflows of a project): the instruction files of each
//! node's working directory, the workflows of each conversation's
//! directory, and each plugin's project search of it, per Host and
//! directory, kept current by the runner's file watch through host access
//! ([`Kept`]). A request takes what is held; only a directory's first
//! search holds it, for at most 5 seconds.

use std::rc::{Rc, Weak};

use demi_backend_host_access::conversation_of;
use demi_backend_host_access::kept::{Found, Kept, SearchError, Subscribe, Wait};
use demi_plugin_interface::{PluginId, WatchSubject};
use demi_runner_protocol::files::Subject;
use demi_shared_types::NodeId;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
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
    /// Whether its tree is loaded.
    loaded: bool,
}

impl Shard {
    /// Where the conversation `id`'s project files are kept, read without
    /// reaching its Host.
    async fn kept_at(&self, id: &ConversationId) -> Result<KeptAt, String> {
        let host = self.host_shard();
        let record = host.owned_conversation(id).await.map_err(|error| error.to_string())?;
        let target = host.resolve_target(&record).await.map_err(|error| error.to_string())?;
        Ok(KeptAt {
            device: target.device().cloned(),
            directory: target.path().to_owned(),
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
            .get(id, &device, cwd.to_owned(), at.loaded, Wait::First)
            .await
            .unwrap_or_else(|| Rc::from([]))
    }

    /// The workflows of the conversation `id`'s directory; none while
    /// nothing was found, such as on a Host that never ran. `wait` says
    /// whether a search under way for a reported change is waited for.
    pub(crate) async fn kept_workflows(
        &self,
        id: &ConversationId,
        wait: Wait,
    ) -> Result<Option<Rc<ProjectWorkflows>>, String> {
        let at = self.kept_at(id).await?;
        let Some(device) = at.device else {
            return Ok(None);
        };
        Ok(self.project_files().workflows.get(id, &device, at.directory, at.loaded, wait).await)
    }

    /// What `plugin`'s project search holds for the conversation of `root`.
    pub(crate) async fn kept_project_search(&self, plugin: &PluginId, root: &NodeId) -> Option<Value> {
        let id = conversation_of(root);
        let at = match self.kept_at(&id).await {
            Ok(at) => at,
            Err(error) => {
                tracing::warn!(conversation = %id, %plugin, %error, "a plugin's project search was not taken");
                return None;
            }
        };
        let key = (plugin.clone(), at.directory);
        self.project_files()
            .plugins
            .get(&id, &at.device?, key, at.loaded, Wait::First)
            .await
    }
}

/// What a search names to watch, as the runner subscribes to it.
fn subjects(watch: &[WatchSubject]) -> Vec<Subject> {
    watch
        .iter()
        .map(|subject| Subject {
            kind: subject.kind,
            path: subject.path.clone(),
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
