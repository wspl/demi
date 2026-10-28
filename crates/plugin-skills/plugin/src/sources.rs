//! User skills: one value per source, keyed by its id, whose files are
//! blobs the value names; the Host directories of the skills that are on;
//! and what the page reads of them.
//!
//! A source is in one place: Global, whose skills every conversation is
//! offered, or a workspace, whose skills its conversations are. A source of
//! a workspace that no longer exists is in no place: [`live`] leaves it out.

use std::collections::{BTreeMap, BTreeSet};

use demi_plugin_interface::{
    DirectoryFile, HostDirectory, PluginError, PluginId, PluginPort, PortFailure, PortRefusal,
    StoredValue, Workspaces,
};
use demi_web_api_protocol::ids::{DeviceId, WorkspaceId};
use demi_shared_types::{B64Bytes, BlobRef, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::fetch::{FetchProblem, Fetched, Skipped};
use crate::heads::Head;
use crate::skill;

/// A source as its value keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Source {
    /// The repository, as the user wrote it.
    pub(crate) origin: String,
    /// Its place in the order the user added sources.
    pub(crate) added: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) fetched_at: Option<Timestamp>,
    pub(crate) skills: Vec<UserSkill>,
    pub(crate) skipped: Vec<Skipped>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) failure: Option<Failure>,
    /// The workspace whose conversations are offered its skills; none for
    /// Global, every conversation's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) workspace: Option<WorkspaceId>,
    /// The paired device that fetches it with its own `git`; none for the
    /// server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) fetcher: Option<DeviceId>,
}

/// A skill of a source's pinned commit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UserSkill {
    pub(crate) name: String,
    pub(crate) description: String,
    /// Its directory in the repository, empty at the root.
    pub(crate) directory: String,
    pub(crate) files: Vec<DirectoryFile>,
    pub(crate) warnings: Vec<String>,
    pub(crate) disable_model_invocation: bool,
    pub(crate) enabled: bool,
}

/// The last fetch's failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub at: Timestamp,
    /// What failed, in English, or in git's or the system's words.
    pub message: String,
    /// What failed, as the page words it; none in a value a fetch before
    /// codes recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<FetchProblem>,
}

impl Source {
    /// The blobs the value names: every file of every skill.
    pub(crate) fn blobs(&self) -> Vec<BlobRef> {
        let blobs: BTreeSet<&BlobRef> = self
            .skills
            .iter()
            .flat_map(|skill| skill.files.iter().map(|file| &file.blob))
            .collect();
        blobs.into_iter().cloned().collect()
    }
}

/// The user's sources, by id, each with the revision its write names.
pub(crate) type Sources = BTreeMap<String, (Source, u64)>;

/// The places a source may be in, as the user's workspaces are now, and the
/// workspace of the request's conversation.
pub(crate) struct Places(Workspaces);

/// What the commands call the place every conversation is offered.
const GLOBAL: &str = "Global";

impl Places {
    pub(crate) fn new(workspaces: Workspaces) -> Self {
        Self(workspaces)
    }

    /// Whether `workspace` is a place: Global, or a workspace that exists.
    pub(crate) fn exists(&self, workspace: Option<&WorkspaceId>) -> bool {
        workspace.is_none_or(|workspace| self.0.workspaces.iter().any(|each| each.id == *workspace))
    }

    /// The place's name as Demi's commands say it.
    pub(crate) fn name(&self, workspace: Option<&WorkspaceId>) -> &str {
        workspace
            .and_then(|workspace| self.0.workspaces.iter().find(|each| each.id == *workspace))
            .map_or(GLOBAL, |each| each.name.as_str())
    }

    /// The workspace of the request's conversation; none for a request of
    /// no conversation.
    pub(crate) fn current(&self) -> Option<&WorkspaceId> {
        self.0.current.as_ref()
    }

    /// Whether the request's conversation is offered the skills of a source
    /// in `workspace`: Global's, and its own workspace's.
    pub(crate) fn offered(&self, workspace: Option<&WorkspaceId>) -> bool {
        workspace.is_none() || workspace == self.current()
    }
}

/// `sources` without those of a workspace that no longer exists, which
/// answer apart, each with its revision, for their values to be removed.
pub(crate) fn live(sources: Sources, places: &Places) -> (Sources, Vec<(String, u64)>) {
    let mut gone = Vec::new();
    let live = sources
        .into_iter()
        .filter_map(|(id, (source, revision))| {
            if places.exists(source.workspace.as_ref()) {
                Some((id, (source, revision)))
            } else {
                gone.push((id, revision));
                None
            }
        })
        .collect();
    (live, gone)
}

/// Every source of the user's.
pub(crate) async fn read(port: &PluginPort) -> Result<Sources, PluginError> {
    port.values()
        .await?
        .into_iter()
        .map(|(id, stored)| Ok((id, decode(stored)?)))
        .collect()
}

/// The source `id`, if the user has it.
pub(crate) async fn find(
    port: &PluginPort,
    id: &str,
) -> Result<Option<(Source, u64)>, PluginError> {
    port.value(id).await?.map(decode).transpose()
}

/// The source `id`; refused when the user has none of that id.
pub(crate) async fn read_one(port: &PluginPort, id: &str) -> Result<(Source, u64), PluginError> {
    find(port, id).await?.ok_or_else(|| not_found(id))
}

fn decode(stored: StoredValue) -> Result<(Source, u64), PluginError> {
    let source = serde_json::from_value(stored.value)
        .map_err(|error| PluginError::failed(format!("a stored source does not read: {error}")))?;
    Ok((source, stored.revision))
}

/// Writes `source` as `id` if it is still at `revision`, none for a new
/// one; [`PortRefusal::Conflict`] when another write came first.
pub(crate) async fn write(
    port: &PluginPort,
    id: &str,
    source: &Source,
    revision: Option<u64>,
) -> Result<u64, PortFailure> {
    let value = serde_json::to_value(source).expect("a source encodes as JSON");
    port.write_value_naming(id, value, revision, source.blobs())
        .await
}

pub(crate) fn not_found(id: &str) -> PluginError {
    PluginError::refused("source_not_found", format!("No skill source \"{id}\""))
}

/// Whether `failure` is another write coming first.
pub(crate) fn conflict(failure: &PortFailure) -> bool {
    matches!(failure, PortFailure::Refused(PortRefusal::Conflict))
}

/// Turns the skills `skills` of source `id` on or off. Turning one on is
/// refused when another skill that is on, in any place, or another of those
/// turned on, already has its name, or the name of its directory: a Host may
/// run conversations of several workspaces. The refusal names the other's
/// source and its place.
pub(crate) fn switch(
    sources: &Sources,
    places: &Places,
    id: &str,
    skills: &BTreeSet<String>,
    enabled: bool,
) -> Result<Source, PluginError> {
    let (source, _) = sources.get(id).ok_or_else(|| not_found(id))?;
    let mut changed = source.clone();
    for name in skills {
        if !changed.skills.iter().any(|skill| skill.name == *name) {
            return Err(PluginError::refused(
                "skill_not_found",
                format!("The source has no skill \"{name}\""),
            ));
        }
    }
    if enabled {
        let mut taken: BTreeMap<String, &Source> = BTreeMap::new();
        for (other, (source, _)) in sources {
            for skill in source.skills.iter().filter(|skill| skill.enabled) {
                if other != id || !skills.contains(&skill.name) {
                    taken.insert(directory_name(&skill.name), source);
                }
            }
        }
        let mut turning: BTreeSet<String> = BTreeSet::new();
        for skill in changed
            .skills
            .iter()
            .filter(|skill| skills.contains(&skill.name))
        {
            let directory = directory_name(&skill.name);
            if let Some(other) = taken.get(&directory) {
                return Err(PluginError::refused(
                    "skill_name_taken",
                    format!(
                        "A skill named \"{}\" from {}, in {}, is on; turn it off first",
                        skill.name,
                        other.origin,
                        places.name(other.workspace.as_ref())
                    ),
                ));
            }
            if !turning.insert(directory) {
                return Err(PluginError::refused(
                    "skill_name_taken",
                    format!(
                        "{} has two skills named \"{}\"; turn on one",
                        source.origin, skill.name
                    ),
                ));
            }
        }
    }
    for skill in changed.skills.iter_mut() {
        if skills.contains(&skill.name) {
            skill.enabled = enabled;
        }
    }
    Ok(changed)
}

/// Source `id` of `sources` after a fetch: the new commit's skills and no
/// failure. `blobs` names each file's bytes, which the fetch put.
///
/// - A skill whose name the source had stays on or off as it was.
/// - A new skill starts on unless it sets `disable-model-invocation`.
/// - Either is on only while no other skill that is on, in another source or
///   earlier in the commit, has its name.
pub(crate) fn fetched(
    sources: &Sources,
    id: &str,
    fetched: &Fetched,
    blobs: &[Vec<BlobRef>],
    at: Timestamp,
) -> Source {
    let (source, _) = &sources[id];
    let mut taken: BTreeSet<String> = sources
        .iter()
        .filter(|(other, _)| *other != id)
        .flat_map(|(_, (other, _))| other.skills.iter())
        .filter(|skill| skill.enabled)
        .map(|skill| directory_name(&skill.name))
        .collect();
    let mut skills = Vec::with_capacity(fetched.skills.len());
    for (skill, blobs) in fetched.skills.iter().zip(blobs) {
        let name = &skill.parsed.name;
        let wanted = match source.skills.iter().find(|old| old.name == *name) {
            Some(old) => old.enabled,
            None => !skill.parsed.disable_model_invocation,
        };
        let enabled = wanted && taken.insert(directory_name(name));
        let files = skill
            .files
            .iter()
            .zip(blobs)
            .map(|(file, blob)| DirectoryFile {
                path: file.path.clone(),
                executable: file.executable,
                blob: blob.clone(),
            })
            .collect();
        skills.push(UserSkill {
            name: name.clone(),
            description: skill.parsed.description.clone(),
            directory: skill.directory.clone(),
            files,
            warnings: skill.parsed.warnings.clone(),
            disable_model_invocation: skill.parsed.disable_model_invocation,
            enabled,
        });
    }
    Source {
        origin: source.origin.clone(),
        added: source.added,
        commit: Some(fetched.commit.clone()),
        fetched_at: Some(at),
        skills,
        skipped: fetched.skipped.clone(),
        failure: None,
        workspace: source.workspace.clone(),
        fetcher: source.fetcher.clone(),
    }
}

/// Puts every file of `fetched`, and answers each skill's files' blobs.
pub(crate) async fn put_files(
    port: &PluginPort,
    fetched: &Fetched,
) -> Result<Vec<Vec<BlobRef>>, PortFailure> {
    let mut blobs = Vec::with_capacity(fetched.skills.len());
    for skill in &fetched.skills {
        let mut files = Vec::with_capacity(skill.files.len());
        for file in &skill.files {
            files.push(port.put_blob(B64Bytes::new(file.bytes.clone())).await?);
        }
        blobs.push(files);
    }
    Ok(blobs)
}

/// The Host directories of the user skills that are on: one per skill,
/// named after it.
pub(crate) fn directories(sources: &Sources) -> Vec<HostDirectory> {
    let mut directories: Vec<HostDirectory> = Vec::new();
    for (source, _) in sources.values() {
        for skill in source.skills.iter().filter(|skill| skill.enabled) {
            directories.push(HostDirectory {
                name: directory_name(&skill.name),
                files: skill.files.clone(),
                workspace: source.workspace.clone(),
            });
        }
    }
    directories
}

/// The directory a skill of `name` is installed in: its name, or for a name
/// that breaks the format's rule, its lowercase letters and digits joined
/// by single hyphens.
pub(crate) fn directory_name(name: &str) -> String {
    if skill::valid_name(name) {
        return name.to_owned();
    }
    let mut directory = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            directory.push(character.to_ascii_lowercase());
        } else if !directory.ends_with('-') && !directory.is_empty() {
            directory.push('-');
        }
    }
    // The format's names have at most 64 characters.
    let directory: String = directory.trim_end_matches('-').chars().take(64).collect();
    if directory.is_empty() {
        "skill".to_owned()
    } else {
        directory.trim_end_matches('-').to_owned()
    }
}

/// Where the `SKILL.md` of a user skill that is on is on every Host.
pub(crate) fn location(plugin: &PluginId, skill: &UserSkill) -> String {
    let directory = HostDirectory {
        name: directory_name(&skill.name),
        files: skill.files.clone(),
        workspace: None,
    };
    format!("{}/SKILL.md", directory.path(plugin))
}

/// The plugin's state for the user's pages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillsState {
    /// Every source, in the order the user added them.
    pub sources: Vec<SourceState>,
}

/// A source as the page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceState {
    pub id: String,
    pub origin: String,
    /// The workspace whose conversations are offered its skills; none for
    /// Global, every conversation's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
    /// The paired device that fetches it; none for the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetcher: Option<DeviceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<Timestamp>,
    pub fetching: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
    /// The repository's default branch points to another commit than the
    /// pinned one, as the last check found.
    pub update_available: bool,
    pub skills: Vec<SkillState>,
    pub skipped: Vec<Skipped>,
}

/// A user skill as the page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillState {
    pub name: String,
    pub description: String,
    pub warnings: Vec<String>,
    pub enabled: bool,
    /// The skill is never offered to the agent.
    pub disable_model_invocation: bool,
    /// While it is off, the origin of the source whose skill that is on
    /// has its name: turning it on would be refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taken_by: Option<String>,
    /// The workspace of that source; none while it is Global's or none
    /// takes the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taken_in: Option<WorkspaceId>,
}

/// The page state of `sources`, `fetching` telling those being fetched and
/// `heads` what is known of each one's default branch.
pub(crate) fn state(
    sources: &Sources,
    fetching: impl Fn(&str) -> bool,
    heads: &BTreeMap<String, Head>,
) -> SkillsState {
    let mut ordered: Vec<(&String, &Source)> = sources
        .iter()
        .map(|(id, (source, _))| (id, source))
        .collect();
    ordered.sort_by_key(|(_, source)| source.added);
    let mut on: BTreeMap<String, &Source> = BTreeMap::new();
    for (_, source) in &ordered {
        for skill in source.skills.iter().filter(|skill| skill.enabled) {
            on.insert(directory_name(&skill.name), source);
        }
    }
    SkillsState {
        sources: ordered
            .into_iter()
            .map(|(id, source)| SourceState {
                id: id.clone(),
                origin: source.origin.clone(),
                workspace: source.workspace.clone(),
                fetcher: source.fetcher.clone(),
                commit: source.commit.clone(),
                fetched_at: source.fetched_at,
                fetching: fetching(id),
                failure: source.failure.clone(),
                update_available: source
                    .commit
                    .as_ref()
                    .zip(heads.get(id).and_then(|head| head.commit.as_ref()))
                    .is_some_and(|(pinned, newest)| pinned != newest),
                skills: source
                    .skills
                    .iter()
                    .map(|skill| {
                        let taker = (!skill.enabled)
                            .then(|| on.get(&directory_name(&skill.name)))
                            .flatten();
                        SkillState {
                            name: skill.name.clone(),
                            description: skill.description.clone(),
                            warnings: skill.warnings.clone(),
                            enabled: skill.enabled,
                            disable_model_invocation: skill.disable_model_invocation,
                            taken_by: taker.map(|taker| taker.origin.clone()),
                            taken_in: taker.and_then(|taker| taker.workspace.clone()),
                        }
                    })
                    .collect(),
                skipped: source.skipped.clone(),
            })
            .collect(),
    }
}
