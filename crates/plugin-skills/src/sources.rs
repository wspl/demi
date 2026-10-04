//! User skills (`skills.md` § User skills, § What the plugin keeps): one
//! value per source, keyed by its id, whose files are blobs the value
//! names; the Host directories of the skills that are on; and what the page
//! reads of them.

use std::collections::{BTreeMap, BTreeSet};

use demi_plugin_interface::{
    DirectoryFile, HostDirectory, PluginError, PluginId, PluginPort, PortFailure, PortRefusal,
    StoredValue,
};
use demi_shared_types::{B64Bytes, BlobRef, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fetch::{Fetched, Skipped};
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
    pub message: String,
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

/// What the instance knows of the commit a source's default branch points
/// to: when it last asked, and what it last heard, if any check or fetch
/// answered (`skills.md` § Updates available).
#[derive(Debug, Clone)]
pub(crate) struct Head {
    pub(crate) checked_at: Timestamp,
    pub(crate) commit: Option<String>,
}

/// The user's sources, by id, each with the revision its write names.
pub(crate) type Sources = BTreeMap<String, (Source, u64)>;

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
/// refused when another skill that is on, or another of those turned on,
/// already has its name, or the name of its directory: the refusal names
/// the other's source.
pub(crate) fn switch(
    sources: &Sources,
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
        let mut taken: BTreeMap<String, &str> = BTreeMap::new();
        for (other, (source, _)) in sources {
            for skill in source.skills.iter().filter(|skill| skill.enabled) {
                if other != id || !skills.contains(&skill.name) {
                    taken.insert(directory_name(&skill.name), &source.origin);
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
            if let Some(origin) = taken.get(&directory) {
                return Err(PluginError::refused(
                    "skill_name_taken",
                    format!(
                        "A skill named \"{}\" from {origin} is on; turn it off first",
                        skill.name
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
/// failure. `blobs` names each file's bytes, which the fetch put. A skill
/// whose name the source had stays on or off as it was; a new one starts on
/// unless it sets `disable-model-invocation`. Either is on only while no
/// other skill that is on, in another source or earlier in the commit, has
/// its name.
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
}

/// The page state of `sources`, `fetching` naming those being fetched and
/// `heads` what is known of each one's default branch.
pub(crate) fn state(
    sources: &Sources,
    fetching: &BTreeSet<String>,
    heads: &BTreeMap<String, Head>,
) -> Value {
    let mut ordered: Vec<(&String, &Source)> = sources
        .iter()
        .map(|(id, (source, _))| (id, source))
        .collect();
    ordered.sort_by_key(|(_, source)| source.added);
    let mut on: BTreeMap<String, &str> = BTreeMap::new();
    for (_, source) in &ordered {
        for skill in source.skills.iter().filter(|skill| skill.enabled) {
            on.insert(directory_name(&skill.name), &source.origin);
        }
    }
    let state = SkillsState {
        sources: ordered
            .into_iter()
            .map(|(id, source)| SourceState {
                id: id.clone(),
                origin: source.origin.clone(),
                commit: source.commit.clone(),
                fetched_at: source.fetched_at,
                fetching: fetching.contains(id),
                failure: source.failure.clone(),
                update_available: source
                    .commit
                    .as_ref()
                    .zip(heads.get(id).and_then(|head| head.commit.as_ref()))
                    .is_some_and(|(pinned, newest)| pinned != newest),
                skills: source
                    .skills
                    .iter()
                    .map(|skill| SkillState {
                        name: skill.name.clone(),
                        description: skill.description.clone(),
                        warnings: skill.warnings.clone(),
                        enabled: skill.enabled,
                        disable_model_invocation: skill.disable_model_invocation,
                        taken_by: (!skill.enabled)
                            .then(|| on.get(&directory_name(&skill.name)))
                            .flatten()
                            .map(|origin| (*origin).to_owned()),
                    })
                    .collect(),
                skipped: source.skipped.clone(),
            })
            .collect(),
    };
    serde_json::to_value(state).expect("the state encodes as JSON")
}
