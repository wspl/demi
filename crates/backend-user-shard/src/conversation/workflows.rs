//! A project's workflows on the conversation's Host: `.demi/workflows/*.yaml`
//! of the nearest directory that has it, from the conversation's working
//! directory up to the root of its git repository, read through the
//! conversation's host access, or through device access for a new conversation
//! before its first message, and parsed with the one parser
//! (`crates/agent-tools/src/workflow.rs`). The shard keeps each conversation
//! directory's workflows current with the runner's file watch
//! (`project_files`), and the spawns, `demi agent workflows`, `demi agent
//! profiles list`, the Workflow menu and the instructions block take what it
//! holds.
//!
//! Why it is built this way:
//! - Nothing applies by itself. A repository is often someone else's, so a
//!   workflow applies only when the user picks it in a conversation's Workflow
//!   menu, never because a repository holds it.
//! - A workflow is a regular file of the project. A `.demi` directory, a
//!   `.demi/workflows` directory or a workflow file that is a link is listed as
//!   unavailable and never followed, since the Host looks at what each entry is
//!   before it opens it; so what a workflow says is always what the repository
//!   holds and what the watch sees.
//! - A spawn takes the workflow as its file is then, without asking the Host,
//!   so a slow or stopped Host never holds a spawn; only a directory's first
//!   read holds them, briefly.
//! - A chosen workflow that no longer reads is shown as unavailable with why,
//!   and spawns fail with that reason until the user picks another, rather than
//!   quietly spawning with other settings.

use demi_agent_tools::{
    ChosenWorkflow, ConversationProfiles, EFFORT_OFF, ListedWorkflow, WORKFLOW_FILE_MAX_BYTES,
    WORKFLOWS_DIRECTORY, Workflow, WorkflowChild, WorkflowListing, WorkflowProfile,
    parse_workflow, workflow_id,
};
use demi_backend_host_access::plugin_files::ReadFilesError;
use demi_backend_host_access::access::HostAccessError;
use demi_backend_host_access::kept::Wait;
use demi_backend_host_access::transition::ChangeRefusal;
use std::rc::Rc;
use std::future::Future;

use demi_backend_host_access::plugin_files::look_files;
use demi_host_interface::{Host as _, HostError, HostPaths};
use demi_plugin_interface::project;
use demi_plugin_interface::{EntryKind, HostFile, HostRead};
use demi_shared_types::{ModelSelection, ReadAt, THINKING_OFF, UnavailableSetting, WorkflowChoice};
use demi_web_api_protocol::conversations::ModelSettings;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use demi_runner_protocol::files::SubscriptionKind as WatchKind;
use demi_web_api_protocol::workflows::{ConversationWorkflows, MenuWorkflow, WatchSubject};
use tokio_util::sync::CancellationToken;

use super::project_files::KeptWorkflows;
use crate::shard::Shard;
use crate::shard::agent_profiles::{Scope, first_offering};

/// Why a workflow directory or file that is a link is not read.
pub(crate) const NOT_LINKS: &str = "Workflows are files of the project, not links.";

/// The workflows a working directory has now.
pub(crate) struct ProjectWorkflows {
    /// The `.demi/workflows` directory they are in, or would be; none when
    /// no directory searched has a `.demi` that holds one.
    pub(crate) directory: Option<String>,
    /// Why that directory cannot be used: it, or its `.demi`, is a link, or
    /// the Host could not tell; none while it can.
    pub(crate) unavailable: Option<String>,
    /// Each file, by id.
    pub(crate) files: Vec<WorkflowFile>,
    /// What the page watches to hear of a workflow added, changed or
    /// removed, or of a nearer directory that gains one.
    pub(crate) watch: Vec<WatchSubject>,
    /// The Host's number before the search.
    pub(crate) read_at: ReadAt,
}

/// One workflow file, as it reads now.
pub(crate) struct WorkflowFile {
    pub(crate) id: String,
    pub(crate) path: String,
    /// The workflow, or why it cannot be used.
    pub(crate) read: Result<Workflow, Unread>,
}

/// Why a workflow file cannot be used.
pub(crate) enum Unread {
    /// It is a link, which is never followed.
    Link,
    /// It does not read, as this says.
    Fails(String),
}

/// The directory a search was asked about is not there, or is no
/// directory, as the Host said at its number.
#[derive(Debug)]
pub(crate) struct NoDirectory(ReadAt);

impl ProjectWorkflows {
    fn get(&self, id: &str) -> Option<&WorkflowFile> {
        self.files.iter().find(|file| file.id == id)
    }

    /// Why the workflow `id` cannot be used: its directory cannot, it does
    /// not read or is a link, or it is gone; none while it can.
    fn unavailable(&self, id: &str) -> Option<String> {
        if let Some(reason) = &self.unavailable {
            return Some(reason.clone());
        }
        match self.get(id) {
            None => Some("it is no longer in the project's .demi/workflows/".to_owned()),
            Some(file) => file.reason(),
        }
    }

    /// The Workflow menu's list, by id (`web-api-conversations.md` § Workflows of a
    /// directory).
    fn menu(&self) -> (ConversationWorkflows, ReadAt) {
        let workflows = self
            .files
            .iter()
            .map(|file| MenuWorkflow {
                id: file.id.clone(),
                name: file.read.as_ref().ok().map(|workflow| workflow.name.clone()),
                path: file.path.clone(),
                unavailable: file.reason(),
                profiles: file
                    .read
                    .as_ref()
                    .map(|workflow| workflow.profiles.iter().map(|profile| profile.id.clone()).collect())
                    .unwrap_or_default(),
            })
            .collect();
        let answer = ConversationWorkflows {
            directory: self.directory.clone(),
            unavailable: self.unavailable.clone(),
            workflows,
            watch: self.watch.clone(),
        };
        (answer, self.read_at.clone())
    }
}

impl WorkflowFile {
    /// Why it cannot be used, in one line, the path first for a file that
    /// does not read.
    fn reason(&self) -> Option<String> {
        match self.read.as_ref().err()? {
            Unread::Link => Some(NOT_LINKS.to_owned()),
            Unread::Fails(error) => {
                let first = error.lines().next().unwrap_or_default();
                let first = first.strip_prefix("error: ").unwrap_or(first);
                Some(format!("{} does not read: {first}", self.path))
            }
        }
    }
}

/// The text of a workflow file the Host answered, read with a limit one
/// byte past [`WORKFLOW_FILE_MAX_BYTES`]: a file larger than that, by the
/// size the Host gave or by the bytes it sent, is refused.
pub(crate) fn workflow_text(answer: HostFile) -> Result<String, String> {
    match answer {
        HostFile::File { size, bytes }
            if size > WORKFLOW_FILE_MAX_BYTES || bytes.as_bytes().len() as u64 > WORKFLOW_FILE_MAX_BYTES =>
        {
            Err("the file is larger than 1 MiB".to_owned())
        }
        HostFile::File { bytes, .. } => {
            String::from_utf8(bytes.as_bytes().to_vec()).map_err(|_| "the file is not UTF-8 text".to_owned())
        }
        HostFile::Unreadable { message } => Err(format!("the Host cannot read it: {message}")),
        HostFile::Directory { .. } => Err("it is a directory".to_owned()),
        HostFile::Link => Err(NOT_LINKS.to_owned()),
        HostFile::Missing | HostFile::Other => Err("there is no such file".to_owned()),
    }
}

/// The workflows of `cwd`, read through `read`: one request finds `cwd`
/// itself, and `.git`, `.demi` and `.demi/workflows` in every directory
/// from `cwd` up to the repository's root, and one more reads the nearest
/// workflows directory's files. Nothing below a searched directory is
/// followed: the Host answers a `.demi`, a `.demi/workflows` or a workflow
/// file that is a link, or that has become one by the time it is read, as
/// a link, and a `.demi` it cannot read makes that directory's workflows
/// unavailable rather than looked past.
pub(crate) async fn search_workflows<E, Fut>(
    paths: HostPaths,
    cwd: &str,
    read: impl Fn(Vec<HostRead>) -> Fut,
) -> Result<Result<ProjectWorkflows, NoDirectory>, E>
where
    Fut: Future<Output = Result<(Vec<HostFile>, ReadAt), E>>,
{
    let ancestors = paths.ancestors(cwd);
    let [demi, workflows] = WORKFLOWS_DIRECTORY;
    let dot_demi = |directory: &str| paths.join(directory, demi);
    let workflows_of = |directory: &str| paths.join(&dot_demi(directory), workflows);
    let mut reads = vec![HostRead::new(cwd.to_owned(), 0)];
    reads.extend(project::root_reads(paths, &ancestors));
    for directory in &ancestors {
        for path in [dot_demi(directory), workflows_of(directory)] {
            reads.push(HostRead {
                path,
                limit: 0,
                within: Some(directory.clone()),
            });
        }
    }
    let (found, read_at) = read(reads).await?;
    let (itself, found) = found.split_at(1);
    if !matches!(itself[0], HostFile::Directory { .. }) {
        return Ok(Err(NoDirectory(read_at)));
    }
    let (git, kept) = found.split_at(ancestors.len());
    let searched = project::searched(&ancestors, git);
    let mut watch = Vec::new();
    let names = |path: String| WatchSubject {
        kind: WatchKind::FolderNames,
        path,
    };
    let mut nearest = None;
    for (directory, answers) in searched.iter().zip(kept.chunks(2)) {
        let unusable = |reason: String| (directory.clone(), workflows_of(directory), Err(reason));
        match (&answers[0], &answers[1]) {
            (HostFile::Missing | HostFile::File { .. } | HostFile::Other, _) => {
                watch.push(names(directory.clone()));
                continue;
            }
            (HostFile::Link, _) => {
                watch.push(names(directory.clone()));
                nearest = Some(unusable(NOT_LINKS.to_owned()));
            }
            (HostFile::Unreadable { message }, _) => {
                watch.push(names(directory.clone()));
                nearest = Some(unusable(format!("the Host cannot read {}: {message}", dot_demi(directory))));
            }
            (HostFile::Directory { .. }, HostFile::Missing | HostFile::File { .. } | HostFile::Other) => {
                watch.push(names(dot_demi(directory)));
                continue;
            }
            (HostFile::Directory { .. }, HostFile::Link) => {
                watch.push(names(dot_demi(directory)));
                nearest = Some(unusable(NOT_LINKS.to_owned()));
            }
            (HostFile::Directory { .. }, HostFile::Unreadable { message }) => {
                watch.push(names(dot_demi(directory)));
                nearest = Some(unusable(format!("the Host cannot read {}: {message}", workflows_of(directory))));
            }
            (HostFile::Directory { .. }, HostFile::Directory { entries }) => {
                watch.push(WatchSubject {
                    kind: WatchKind::FolderDetails,
                    path: workflows_of(directory),
                });
                nearest = Some((directory.clone(), workflows_of(directory), Ok(entries.clone())));
            }
        }
        break;
    }
    let Some((project, directory, entries)) = nearest else {
        return Ok(Ok(ProjectWorkflows {
            directory: None,
            unavailable: None,
            files: Vec::new(),
            watch,
            read_at,
        }));
    };
    let entries = match entries {
        Ok(entries) => entries,
        Err(reason) => {
            return Ok(Ok(ProjectWorkflows {
                directory: Some(directory),
                unavailable: Some(reason),
                files: Vec::new(),
                watch,
                read_at,
            }));
        }
    };
    let mut named: Vec<(String, String, bool)> = entries
        .iter()
        .filter(|entry| matches!(entry.kind, EntryKind::File | EntryKind::Symlink))
        .filter_map(|entry| {
            let id = workflow_id(&entry.name)?;
            Some((id.to_owned(), paths.join(&directory, &entry.name), entry.kind == EntryKind::Symlink))
        })
        .collect();
    named.sort();
    let reads: Vec<HostRead> = named
        .iter()
        .filter(|(_, _, link)| !link)
        .map(|(_, path, _)| HostRead {
            path: path.clone(),
            // One byte past the largest file read, to tell a larger one.
            limit: WORKFLOW_FILE_MAX_BYTES + 1,
            within: Some(project.clone()),
        })
        .collect();
    let mut answers = if reads.is_empty() {
        Vec::new()
    } else {
        read(reads).await?.0
    }
    .into_iter();
    let files = named
        .into_iter()
        .map(|(id, path, link)| {
            let read = match if link { HostFile::Link } else { answers.next().unwrap_or(HostFile::Missing) } {
                HostFile::Link => Err(Unread::Link),
                answer => workflow_text(answer)
                    .and_then(|text| parse_workflow(&text).map_err(|error| error.to_string()))
                    .map_err(Unread::Fails),
            };
            WorkflowFile { id, path, read }
        })
        .collect();
    Ok(Ok(ProjectWorkflows {
        directory: Some(directory),
        unavailable: None,
        files,
        watch,
        read_at,
    }))
}

/// Why a device's directory's workflows were not listed.
#[derive(Debug)]
pub enum DeviceWorkflowsRefusal {
    /// The device's runner is not connected.
    Offline,
    /// The directory is not there.
    NoDirectory,
    Host(HostError),
}

/// The workflows of `directory`, the conversation `id`'s, read now through
/// its host access in the form that never wakes its Host. A directory that
/// is gone has none, and is watched for coming back.
pub(crate) async fn search_conversation_workflows(
    shard: &Shard,
    id: &ConversationId,
    device: &DeviceId,
    directory: &str,
) -> Result<ProjectWorkflows, ReadFilesError> {
    let paths = shard
        .host_paths(id)
        .await
        .map_err(|error| ReadFilesError::Access(error.into()))?;
    let host = shard.host_shard();
    let cancel = CancellationToken::new();
    let cancel = &cancel;
    let found = search_workflows(paths, directory, |reads| async move {
        host.read_files_on(id, Some(device), &reads, cancel).await
    })
    .await?;
    Ok(found.unwrap_or_else(|NoDirectory(read_at)| ProjectWorkflows {
        directory: None,
        unavailable: None,
        files: Vec::new(),
        watch: vec![WatchSubject {
            kind: WatchKind::FolderNames,
            path: directory.to_owned(),
        }],
        read_at,
    }))
}

impl Shard {
    /// The workflows the shard holds for the conversation `id`'s working
    /// directory; why none are, while it holds none, as on a Host that has
    /// not run since. `wait` says whether a search under way for a change
    /// the runner reported is waited for.
    pub(crate) async fn project_workflows(&self, id: &ConversationId, wait: Wait) -> Result<Rc<ProjectWorkflows>, String> {
        match self.kept_workflows(id, wait).await.map_err(|error| error.to_string())? {
            KeptWorkflows::Found(found) => Ok(found),
            KeptWorkflows::None { .. } => Err(ReadFilesError::NotRunning.to_string()),
        }
    }

    /// The Workflow menu's list for a new conversation that works in
    /// `directory` of the paired device `device`, before its first message
    /// gives it a record: read
    /// through device access, which wakes nothing; none while the device's
    /// runner is not connected. With `after`, the answer is read at that
    /// number or later (`web-api-conversations.md` § Workflows of a
    /// directory).
    pub async fn device_workflows(
        &self,
        device: &DeviceId,
        directory: &str,
        after: Option<&ReadAt>,
    ) -> Result<(ConversationWorkflows, ReadAt), DeviceWorkflowsRefusal> {
        // What a loaded conversation there keeps already, unless it was read
        // before `after`: the search below reads it anew.
        if let Some(found) = self.project_files().workflows.held(device, directory.to_owned(), after) {
            return Ok(found.menu());
        }
        let host = self.devices().device_access(device).ok_or(DeviceWorkflowsRefusal::Offline)?;
        let paths = host.identity().paths;
        let host = &host;
        let found = search_workflows(paths, directory, |reads| async move { look_files(host, &reads).await })
            .await
            .map_err(DeviceWorkflowsRefusal::Host)?
            .map_err(|NoDirectory(_)| DeviceWorkflowsRefusal::NoDirectory)?;
        Ok(found.menu())
    }

    /// The Workflow menu's list for the conversation `id` (`GET
    /// /conversations/:id/workflows`), with what the page watches. The page
    /// asks when the runner reported a change, so a search under way for one
    /// is waited for; with `after`, as its watch went live after the answer
    /// it holds, one read at that number or later is. While no search is
    /// held, it answers no workflows, read at no number, and the working
    /// directory to watch: the page's watch going live there, as the Host
    /// runs, makes that answer out of date (`web-api-conversations.md`
    /// § Workflows of a directory).
    pub async fn menu_workflows(
        &self,
        id: &ConversationId,
        after: Option<ReadAt>,
    ) -> Result<(ConversationWorkflows, Option<ReadAt>), Rc<HostAccessError>> {
        let wait = after.map_or(Wait::Current, Wait::After);
        Ok(match self.kept_workflows(id, wait).await? {
            KeptWorkflows::Found(found) => {
                let (answer, read_at) = found.menu();
                (answer, Some(read_at))
            }
            KeptWorkflows::None { directory } => {
                let answer = ConversationWorkflows {
                    directory: None,
                    unavailable: None,
                    workflows: Vec::new(),
                    watch: vec![WatchSubject {
                        kind: WatchKind::FolderNames,
                        path: directory,
                    }],
                };
                (answer, None)
            }
        })
    }

    /// `demi agent workflows list` for the conversation `id`.
    pub(crate) async fn workflow_listing(&self, id: &ConversationId) -> Result<WorkflowListing, String> {
        let chosen = self.workflow_choice(id).await?;
        let found = self
            .project_workflows(id, Wait::First)
            .await
            .map_err(|error| format!("the project's workflows cannot be read: {error}"))?;
        let workflows = found
            .files
            .iter()
            .map(|file| ListedWorkflow {
                id: file.id.clone(),
                name: file.read.as_ref().ok().map(|workflow| workflow.name.clone()),
                path: file.path.clone(),
                unavailable: file.read.as_ref().err().map(|unread| match unread {
                    Unread::Link => NOT_LINKS.to_owned(),
                    Unread::Fails(error) => error.clone(),
                }),
            })
            .collect();
        Ok(WorkflowListing {
            chosen,
            directory: found.directory.clone(),
            unavailable: found.unavailable.clone(),
            workflows,
        })
    }

    /// The Workflow menu choice of the conversation `id`.
    pub(crate) async fn workflow_choice(&self, id: &ConversationId) -> Result<WorkflowChoice, String> {
        let record = self
            .services()
            .control
            .conversation(id.clone())
            .await
            .map_err(|error| error.to_string())?
            .ok_or("the conversation is gone")?;
        Ok(record.workflow)
    }

    /// The profiles the spawns of the conversation `id` use now: the
    /// user's settings, none, or the chosen workflow's as its file reads,
    /// each profile's model looked for in the user's provider entries in
    /// their order.
    pub(crate) async fn conversation_profiles(
        &self,
        id: &ConversationId,
    ) -> Result<ConversationProfiles, String> {
        let chosen = match self.workflow_choice(id).await? {
            WorkflowChoice::Default => return Ok(ConversationProfiles::Settings),
            WorkflowChoice::None => return Ok(ConversationProfiles::Off),
            WorkflowChoice::Workflow { id } => id,
        };
        let found = self.project_workflows(id, Wait::First).await?;
        if let Some(reason) = found.unavailable(&chosen) {
            return Ok(ConversationProfiles::Unavailable { id: chosen, reason });
        }
        let file = found.get(&chosen).expect("an available workflow is listed");
        let workflow = file.read.as_ref().ok().expect("an available workflow reads");
        let scope = self.scope().await.map_err(|error| error.to_string())?;
        let mut profiles = Vec::with_capacity(workflow.profiles.len());
        for profile in &workflow.profiles {
            let (model, context_limit) = match &profile.model {
                None => (None, None),
                Some(model) => {
                    let chosen = self.workflow_selection(&scope, profile, model, &file.path).await;
                    match chosen.and_then(|selection| {
                        let tokens = workflow_context(profile, &selection, &file.path)?;
                        Ok((selection, tokens))
                    }) {
                        Ok((selection, tokens)) => (Some(Ok(selection)), tokens),
                        Err(missing) => (Some(Err(missing)), None),
                    }
                }
            };
            profiles.push(WorkflowChild {
                name: profile.id.clone(),
                description: profile.description.clone(),
                instructions: profile.instructions.clone(),
                can_spawn: profile.spawning,
                model,
                context_limit,
            });
        }
        Ok(ConversationProfiles::Workflow(ChosenWorkflow {
            id: chosen,
            name: workflow.name.clone(),
            profiles,
        }))
    }

    /// The selection a workflow profile's model `model` makes now: in the
    /// first of the user's entries that offers it, with its effort and
    /// tier; or what is missing, which the file at `path` changes.
    async fn workflow_selection(
        &self,
        scope: &Scope,
        profile: &WorkflowProfile,
        model: &str,
        path: &str,
    ) -> Result<ModelSelection, String> {
        let Some(provider_id) = first_offering(scope, model) else {
            return Err(format!(
                "no provider entry of the user's offers model \"{model}\"; change the profile's model in {path}, or ask the user to add a provider that offers it"
            ));
        };
        let chosen = ModelSettings {
            provider_id,
            model_id: model.to_owned(),
            thinking_effort: profile.effort.as_ref().map(|effort| {
                if effort == EFFORT_OFF {
                    THINKING_OFF.to_owned()
                } else {
                    effort.clone()
                }
            }),
            service_tier_id: profile.tier.clone(),
        };
        self.chosen_selection(&chosen).await.map_err(|refusal| match refusal {
            ChangeRefusal::SettingUnavailable(UnavailableSetting::Effort(effort)) => format!(
                "model \"{model}\" does not offer the effort \"{effort}\"; change the profile's effort in {path}"
            ),
            ChangeRefusal::SettingUnavailable(UnavailableSetting::Tier(tier)) => format!(
                "model \"{model}\" does not offer the service tier \"{tier}\"; change the profile's tier in {path}"
            ),
            other => format!("model \"{model}\" cannot be used: {other}"),
        })
    }

    /// The text of the file `path`, read against `directory`, on the
    /// conversation `id`'s Host.
    pub(crate) async fn workflow_file_text(
        &self,
        id: &ConversationId,
        directory: &str,
        path: &str,
    ) -> Result<String, String> {
        let paths = self.host_paths(id).await.map_err(|error| error.to_string())?;
        // The Host reads the path against the directory as a shell there
        // does, following links before `..`.
        let read = HostRead {
            path: paths.beside(directory, path),
            limit: WORKFLOW_FILE_MAX_BYTES + 1,
    within: None,
};
        let (mut found, _) = self
            .host_shard()
            .read_files(id, std::slice::from_ref(&read), &CancellationToken::new())
            .await
            .map_err(|error| error.to_string())?;
        match found.pop() {
            Some(HostFile::Missing | HostFile::Other) | None => Err(format!("no file {path}")),
            Some(answer) => workflow_text(answer).map_err(|reason| format!("{path}: {reason}")),
        }
    }
}

/// The tokens of the Context a workflow's profile names for its model's
/// selection `selection`, none for the user's limit; or why the model does
/// not offer it, which the file at `path` changes.
fn workflow_context(
    profile: &WorkflowProfile,
    selection: &ModelSelection,
    path: &str,
) -> Result<Option<u32>, String> {
    let window = selection.model.context_window;
    let tokens = profile.context.and_then(|choice| choice.tokens(window));
    demi_backend_accounts::subagents::check_context(Some((&selection.model.id, window)), tokens)
        .map_err(|refusal| format!("{refusal}; change the profile's context in {path}"))?;
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use demi_shared_types::B64Bytes;

    use super::*;

    /// A Host that says a file is small but sends more than the limit, as
    /// a file that grew between its size and its read: the bytes decide.
    #[test]
    fn a_file_whose_bytes_pass_the_limit_is_refused_whatever_size_the_host_gave() {
        let limit = usize::try_from(WORKFLOW_FILE_MAX_BYTES).unwrap();
        let answer = |bytes: usize, size: u64| HostFile::File {
            bytes: B64Bytes::new(vec![b'a'; bytes]),
            size,
        };
        assert_eq!(workflow_text(answer(limit + 1, 10)), Err("the file is larger than 1 MiB".to_owned()));
        assert_eq!(workflow_text(answer(10, WORKFLOW_FILE_MAX_BYTES + 1)), Err("the file is larger than 1 MiB".to_owned()));
        assert_eq!(workflow_text(answer(limit, WORKFLOW_FILE_MAX_BYTES)).map(|text| text.len()), Ok(limit));
    }
}
