//! A project's workflows on the conversation's Host (`subagents.md`
//! § Workflows of a project): `.demi/workflows/*.yaml` of the nearest
//! directory that has it, from the conversation's working directory up to
//! the root of its git repository, read through the conversation's host
//! access, or through device access for a new conversation before its
//! first message, and parsed with the one parser. The spawns, `demi agent
//! workflows`, the Workflow menu and the instructions block read them here,
//! as the files are at that moment.

use demi_agent_tools::{
    ChosenWorkflow, ConversationProfiles, EFFORT_OFF, ListedWorkflow, WORKFLOW_FILE_MAX_BYTES,
    WORKFLOWS_DIRECTORY, Workflow, WorkflowChild, WorkflowListing, WorkflowProfile,
    parse_workflow, workflow_id,
};
use demi_backend_host_access::plugin_files::ReadFilesError;
use demi_backend_host_access::transition::ChangeRefusal;
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

use crate::shard::Shard;
use crate::shard::agent_profiles::{Scope, first_offering};

/// Why a workflow directory or file that is a symbolic link is not read.
pub(crate) const NOT_LINKS: &str = "Workflows are files of the project, not links.";

/// The workflows a working directory has now.
pub(crate) struct ProjectWorkflows {
    /// The `.demi/workflows` directory they are in; none when no directory
    /// searched has one.
    pub(crate) directory: Option<String>,
    /// Why that directory cannot be used, a link; none while it can.
    pub(crate) unavailable: Option<&'static str>,
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
    /// It is a symbolic link, which is never followed.
    Link,
    /// It does not read, as this says.
    Fails(String),
}

impl ProjectWorkflows {
    fn get(&self, id: &str) -> Option<&WorkflowFile> {
        self.files.iter().find(|file| file.id == id)
    }

    /// Why the workflow `id` cannot be used: its directory or file is a
    /// link, it does not read, or it is gone; none while it can.
    fn unavailable(&self, id: &str) -> Option<String> {
        if let Some(reason) = self.unavailable {
            return Some(reason.to_owned());
        }
        match self.get(id) {
            None => Some("it is no longer in the project's .demi/workflows/".to_owned()),
            Some(file) => file.reason(),
        }
    }

    /// The Workflow menu's list, by id.
    fn menu(self) -> (ConversationWorkflows, ReadAt) {
        let workflows = self
            .files
            .iter()
            .map(|file| MenuWorkflow {
                id: file.id.clone(),
                name: file.read.as_ref().ok().map(|workflow| workflow.name.clone()),
                error: file.reason(),
            })
            .collect();
        let answer = ConversationWorkflows {
            directory: self.directory,
            unavailable: self.unavailable.map(str::to_owned),
            workflows,
            watch: self.watch,
        };
        (answer, self.read_at)
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
        HostFile::Missing | HostFile::Other => Err("there is no such file".to_owned()),
    }
}

/// The workflows of `cwd`, read through `read`: one request finds `.git`,
/// `.demi` and `.demi/workflows` in every directory from `cwd` up to the
/// repository's root, and one more reads the nearest directory's files. A
/// `.demi/workflows` that is a link, and a workflow file that is one, are
/// never followed (`subagents.md` § Workflows of a project).
pub(crate) async fn search_workflows<E, Fut>(
    paths: HostPaths,
    cwd: &str,
    read: impl Fn(Vec<HostRead>) -> Fut,
) -> Result<ProjectWorkflows, E>
where
    Fut: Future<Output = Result<(Vec<HostFile>, ReadAt), E>>,
{
    let ancestors = paths.ancestors(cwd);
    let [demi, workflows] = WORKFLOWS_DIRECTORY;
    let dot_demi = |directory: &str| paths.join(directory, demi);
    let workflows_of = |directory: &str| paths.join(&dot_demi(directory), workflows);
    let mut reads = project::root_reads(paths, &ancestors);
    for directory in &ancestors {
        for path in [dot_demi(directory), workflows_of(directory)] {
            reads.push(HostRead { path, limit: 0 });
        }
    }
    let (found, read_at) = read(reads).await?;
    let (git, kept) = found.split_at(ancestors.len());
    let searched = project::searched(&ancestors, git);
    let mut watch = Vec::new();
    let mut nearest = None;
    for (directory, answers) in searched.iter().zip(kept.chunks(2)) {
        // `.demi`'s own listing says what `workflows` is without following it.
        let listed = match &answers[0] {
            HostFile::Directory { entries } => entries.iter().find(|entry| entry.name == workflows),
            _ => None,
        };
        if listed.is_some_and(|entry| entry.kind == EntryKind::Symlink) {
            watch.push(WatchSubject {
                kind: WatchKind::FolderNames,
                path: dot_demi(directory),
            });
            nearest = Some((workflows_of(directory), None));
            break;
        }
        if let HostFile::Directory { entries } = &answers[1] {
            let path = workflows_of(directory);
            watch.push(WatchSubject {
                kind: WatchKind::FolderDetails,
                path: path.clone(),
            });
            nearest = Some((path, Some(entries.clone())));
            break;
        }
        let path = if matches!(answers[0], HostFile::Directory { .. }) {
            dot_demi(directory)
        } else {
            directory.clone()
        };
        watch.push(WatchSubject {
            kind: WatchKind::FolderNames,
            path,
        });
    }
    let (directory, entries) = match nearest {
        None => (None, None),
        Some((directory, entries)) => (Some(directory), entries),
    };
    let Some(entries) = entries else {
        return Ok(ProjectWorkflows {
            unavailable: directory.is_some().then_some(NOT_LINKS),
            directory,
            files: Vec::new(),
            watch,
            read_at,
        });
    };
    let directory = directory.expect("a directory with entries");
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
            let read = if link {
                Err(Unread::Link)
            } else {
                let answer = answers.next().unwrap_or(HostFile::Missing);
                workflow_text(answer)
                    .and_then(|text| parse_workflow(&text).map_err(|error| error.to_string()))
                    .map_err(Unread::Fails)
            };
            WorkflowFile { id, path, read }
        })
        .collect();
    Ok(ProjectWorkflows {
        directory: Some(directory),
        unavailable: None,
        files,
        watch,
        read_at,
    })
}

impl Shard {
    /// The workflows of the conversation `id`'s working directory, read now
    /// through its host access in the form that never wakes its Host.
    pub(crate) async fn project_workflows(
        &self,
        id: &ConversationId,
        cancel: &CancellationToken,
    ) -> Result<ProjectWorkflows, ReadFilesError> {
        let paths = self
            .host_paths(id)
            .await
            .map_err(|error| ReadFilesError::Access(error.into()))?;
        let cwd = self.placement(id).await?.directory;
        let host = self.host_shard();
        search_workflows(paths, &cwd, |reads| async move { host.read_files(id, &reads, cancel).await }).await
    }

    /// The Workflow menu's list for a new conversation that works in
    /// `directory` of the paired device `device`, before its first message
    /// gives it a record (`subagents.md` § Workflows of a project): read
    /// through device access, which wakes nothing; none while the device's
    /// runner is not connected.
    pub async fn device_workflows(
        &self,
        device: &DeviceId,
        directory: &str,
    ) -> Option<Result<(ConversationWorkflows, ReadAt), HostError>> {
        let host = self.devices().device_access(device)?;
        let paths = host.identity().paths;
        let host = &host;
        let found = search_workflows(paths, directory, |reads| async move { look_files(host, &reads).await }).await;
        Some(found.map(ProjectWorkflows::menu))
    }

    /// The Workflow menu's list for the conversation `id` (`GET
    /// /conversations/:id/workflows`), with what the page watches.
    pub async fn menu_workflows(
        &self,
        id: &ConversationId,
    ) -> Result<(ConversationWorkflows, ReadAt), ReadFilesError> {
        Ok(self.project_workflows(id, &CancellationToken::new()).await?.menu())
    }

    /// `demi agent workflows list` for the conversation `id`.
    pub(crate) async fn workflow_listing(&self, id: &ConversationId) -> Result<WorkflowListing, String> {
        let chosen = self.workflow_choice(id).await?;
        let found = self
            .project_workflows(id, &CancellationToken::new())
            .await
            .map_err(|error| format!("the project's workflows cannot be read: {error}"))?;
        let workflows = found
            .files
            .iter()
            .map(|file| ListedWorkflow {
                id: file.id.clone(),
                name: file.read.as_ref().ok().map(|workflow| workflow.name.clone()),
                error: file.read.as_ref().err().map(|unread| match unread {
                    Unread::Link => NOT_LINKS.to_owned(),
                    Unread::Fails(error) => error.clone(),
                }),
            })
            .collect();
        Ok(WorkflowListing {
            chosen,
            directory: found.directory,
            unavailable: found.unavailable.map(str::to_owned),
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
        let found = self
            .project_workflows(id, &CancellationToken::new())
            .await
            .map_err(|error| error.to_string())?;
        if let Some(reason) = found.unavailable(&chosen) {
            return Ok(ConversationProfiles::Unavailable { id: chosen, reason });
        }
        let file = found.get(&chosen).expect("an available workflow is listed");
        let workflow = file.read.as_ref().ok().expect("an available workflow reads");
        let scope = self.scope().await.map_err(|error| error.to_string())?;
        let mut profiles = Vec::with_capacity(workflow.profiles.len());
        for profile in &workflow.profiles {
            let model = match &profile.model {
                None => None,
                Some(model) => Some(self.workflow_selection(&scope, profile, model, &file.path).await),
            };
            profiles.push(WorkflowChild {
                name: profile.id.clone(),
                description: profile.description.clone(),
                instructions: profile.instructions.clone(),
                can_spawn: profile.spawning,
                model,
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
