//! A project's workflows on the conversation's Host (`subagents.md`
//! § Workflows of a project): `.demi/workflows/*.yaml` of the nearest
//! directory that has it, from the conversation's working directory up to
//! the root of its git repository, read through the conversation's host
//! access and parsed with the one parser. The spawns, `demi agent
//! workflows`, the Workflow menu and the instructions block read them here,
//! as the files are at that moment.

use demi_agent_tools::{
    ChosenWorkflow, ConversationProfiles, EFFORT_OFF, ListedWorkflow, WORKFLOW_FILE_MAX_BYTES,
    WORKFLOWS_DIRECTORY, Workflow, WorkflowChild, WorkflowListing, WorkflowProfile,
    parse_workflow, workflow_id,
};
use demi_backend_host_access::plugin_files::ReadFilesError;
use demi_backend_host_access::transition::ChangeRefusal;
use demi_plugin_interface::project;
use demi_plugin_interface::{EntryKind, HostFile, HostRead};
use demi_shared_types::{ModelSelection, ReadAt, THINKING_OFF, UnavailableSetting, WorkflowChoice};
use demi_web_api_protocol::conversations::ModelSettings;
use demi_web_api_protocol::ids::ConversationId;
use demi_runner_protocol::files::SubscriptionKind as WatchKind;
use demi_web_api_protocol::workflows::{ConversationWorkflows, MenuWorkflow, WatchSubject};
use tokio_util::sync::CancellationToken;

use crate::shard::Shard;
use crate::shard::agent_profiles::{Scope, first_offering};

/// The workflows a conversation's working directory has now.
pub(crate) struct ProjectWorkflows {
    /// The `.demi/workflows` directory they are in; none when no directory
    /// searched has one.
    pub(crate) directory: Option<String>,
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
    /// The workflow, or why the file does not read.
    pub(crate) read: Result<Workflow, String>,
}

impl ProjectWorkflows {
    fn get(&self, id: &str) -> Option<&WorkflowFile> {
        self.files.iter().find(|file| file.id == id)
    }
}

impl WorkflowFile {
    /// Why it does not read, in one line, the path first.
    fn reason(&self) -> Option<String> {
        self.read.as_ref().err().map(|error| {
            let first = error.lines().next().unwrap_or_default();
            let first = first.strip_prefix("error: ").unwrap_or(first);
            format!("{} does not read: {first}", self.path)
        })
    }
}

impl Shard {
    /// The workflows of the conversation `id`'s working directory, read now
    /// in the form that never wakes its Host: one request finds `.git`,
    /// `.demi` and `.demi/workflows` in every directory from the working
    /// directory up, and one more reads the nearest directory's files.
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
        let ancestors = paths.ancestors(&cwd);
        let [demi, workflows] = WORKFLOWS_DIRECTORY;
        let dot_demi = |directory: &str| paths.join(directory, demi);
        let workflows_of = |directory: &str| paths.join(&dot_demi(directory), workflows);
        let mut reads = project::root_reads(paths, &ancestors);
        for directory in &ancestors {
            for path in [dot_demi(directory), workflows_of(directory)] {
                reads.push(HostRead { path, limit: 0 });
            }
        }
        let (found, read_at) = self.host_shard().read_files(id, &reads, cancel).await?;
        let (git, kept) = found.split_at(ancestors.len());
        let searched = project::searched(&ancestors, git);
        let mut watch = Vec::new();
        let mut nearest = None;
        for (directory, answers) in searched.iter().zip(kept.chunks(2)) {
            let is_directory = |answer: &HostFile| matches!(answer, HostFile::Directory { .. });
            if let HostFile::Directory { entries } = &answers[1] {
                let path = workflows_of(directory);
                watch.push(WatchSubject {
                    kind: WatchKind::FolderDetails,
                    path: path.clone(),
                });
                nearest = Some((path, entries.clone()));
                break;
            }
            let (kind, path) = if is_directory(&answers[0]) {
                (WatchKind::FolderNames, dot_demi(directory))
            } else {
                (WatchKind::FolderNames, directory.clone())
            };
            watch.push(WatchSubject { kind, path });
        }
        let Some((directory, entries)) = nearest else {
            return Ok(ProjectWorkflows {
                directory: None,
                files: Vec::new(),
                watch,
                read_at,
            });
        };
        let mut named: Vec<(String, String)> = entries
            .iter()
            .filter(|entry| matches!(entry.kind, EntryKind::File | EntryKind::Symlink))
            .filter_map(|entry| {
                workflow_id(&entry.name).map(|id| (id.to_owned(), paths.join(&directory, &entry.name)))
            })
            .collect();
        named.sort();
        let reads: Vec<HostRead> = named
            .iter()
            .map(|(_, path)| HostRead {
                path: path.clone(),
                // One byte past the largest file read, to tell a larger one.
                limit: WORKFLOW_FILE_MAX_BYTES + 1,
            })
            .collect();
        let answers = if reads.is_empty() {
            Vec::new()
        } else {
            self.host_shard().read_files(id, &reads, cancel).await?.0
        };
        let files = named
            .into_iter()
            .zip(answers)
            .map(|((id, path), answer)| {
                let read = match answer {
                    HostFile::File { size, .. } if size > WORKFLOW_FILE_MAX_BYTES => {
                        Err("the file is larger than 1 MiB".to_owned())
                    }
                    HostFile::File { bytes, .. } => match std::str::from_utf8(bytes.as_bytes()) {
                        Ok(text) => parse_workflow(text).map_err(|error| error.to_string()),
                        Err(_) => Err("the file is not UTF-8 text".to_owned()),
                    },
                    HostFile::Unreadable { message } => Err(format!("the Host cannot read it: {message}")),
                    // Gone or changed into something else since the listing.
                    HostFile::Missing | HostFile::Directory { .. } | HostFile::Other => {
                        Err("the file is no longer there".to_owned())
                    }
                };
                WorkflowFile { id, path, read }
            })
            .collect();
        Ok(ProjectWorkflows {
            directory: Some(directory),
            files,
            watch,
            read_at,
        })
    }

    /// The Workflow menu's list for the conversation `id` (`GET
    /// /conversations/:id/workflows`), with what the page watches.
    pub async fn menu_workflows(
        &self,
        id: &ConversationId,
    ) -> Result<(ConversationWorkflows, ReadAt), ReadFilesError> {
        let found = self.project_workflows(id, &CancellationToken::new()).await?;
        let workflows = found
            .files
            .iter()
            .map(|file| MenuWorkflow {
                id: file.id.clone(),
                name: file.read.as_ref().ok().map(|workflow| workflow.name.clone()),
                error: file.reason(),
            })
            .collect();
        let answer = ConversationWorkflows {
            directory: found.directory,
            workflows,
            watch: found.watch,
        };
        Ok((answer, found.read_at))
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
                error: file.read.as_ref().err().cloned(),
            })
            .collect();
        Ok(WorkflowListing {
            chosen,
            directory: found.directory,
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
        let Some(file) = found.get(&chosen) else {
            return Ok(ConversationProfiles::Unavailable {
                id: chosen,
                reason: "it is no longer in the project's .demi/workflows/".to_owned(),
            });
        };
        let workflow = match &file.read {
            Ok(workflow) => workflow,
            Err(_) => {
                return Ok(ConversationProfiles::Unavailable {
                    reason: file.reason().unwrap_or_default(),
                    id: chosen,
                });
            }
        };
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
        let read = HostRead {
            path: paths.resolve(directory, path),
            limit: WORKFLOW_FILE_MAX_BYTES + 1,
        };
        let (mut found, _) = self
            .host_shard()
            .read_files(id, std::slice::from_ref(&read), &CancellationToken::new())
            .await
            .map_err(|error| error.to_string())?;
        match found.pop() {
            Some(HostFile::File { size, .. }) if size > WORKFLOW_FILE_MAX_BYTES => {
                Err(format!("{path} is larger than 1 MiB"))
            }
            Some(HostFile::File { bytes, .. }) => String::from_utf8(bytes.as_bytes().to_vec())
                .map_err(|_| format!("{path} is not UTF-8 text")),
            Some(HostFile::Directory { .. }) => Err(format!("{path} is a directory")),
            Some(HostFile::Unreadable { message }) => Err(format!("{path} cannot be read: {message}")),
            Some(HostFile::Missing | HostFile::Other) | None => Err(format!("no file {path}")),
        }
    }
}
