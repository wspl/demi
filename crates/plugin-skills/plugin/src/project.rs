//! Project skills: the skills a project carries in `.agents/skills` and
//! `.claude/skills` of its directory, and nowhere else, found through Host
//! reads that never wake the Host.
//!
//! Each round of the search reads every path it needs in one request. The
//! backend keeps the result and searches again when what the search names to
//! watch changes: the names in the skills directories, the coming of one that
//! is not there yet, and each skill's `SKILL.md`.
//!
//! Project skills are always available: they come with the project, their files
//! stay where they are, and the catalog names each `SKILL.md` at its own path.
//! Demi does not check them first: the user chose the directory, and the agent
//! runs its code in any case.
//!
//! Only these two directories of the project's own directory are read, never
//! one above it, a subagent's own working directory, the user's home or another
//! agent's configuration.

use demi_host_interface::HostPaths;
use demi_plugin_interface::{
    EntryKind, HostFile, HostRead, PluginPort, PortFailure, SubscriptionKind, WatchSubject,
};
use serde::{Deserialize, Serialize};

use crate::skill::{self, SKILL_MD_MAX_BYTES};

/// The most directories one search reads.
const MAX_DIRECTORIES: usize = 2_000;

/// How deep below a skills directory a search looks.
const MAX_DEPTH: usize = 6;

/// The most project skills one search finds.
const MAX_SKILLS: usize = 100;

/// The skills directories of the project, the first over the second.
const SKILLS_DIRECTORIES: [&str; 2] = [".agents/skills", ".claude/skills"];

/// A project skill, listed at its own path on the Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectSkill {
    pub(crate) name: String,
    pub(crate) description: String,
    /// The path of its `SKILL.md` on the Host.
    pub(crate) location: String,
    pub(crate) disable_model_invocation: bool,
}

/// The project skills of `directory`, one per name, `.agents/skills` over
/// `.claude/skills`, with what tells of a change to them.
///
/// What tells of a change is:
///
/// - the names of every directory the search listed;
/// - the coming of a skills directory that is not there, or of the folder it
///   is in, `.agents` or `.claude`, when that is missing too: a subscription
///   to a missing path watches its folder, which must be there, and one to
///   the project directory's names would count every file made in it;
/// - every `SKILL.md` it looked for below them, found, missing or not a skill,
///   so a manifest created in an empty skill directory, a skill added to a
///   group or a manifest repaired is seen.
///
/// A skill's own directory is not listed, so a file added beside its `SKILL.md`
/// changes nothing. What is not a skill, has warnings or is shadowed is logged.
///
/// `paths` is how the Host writes its paths, which every path the search reads
/// is built in.
pub(crate) async fn search(
    port: &PluginPort,
    paths: HostPaths,
    directory: &str,
) -> Result<(Vec<ProjectSkill>, Vec<WatchSubject>), PortFailure> {
    let join = |directory: &str, name: &str| paths.join(directory, name);
    let above = |skills: &str| paths.parent(skills).expect("a skills directory is in a folder");
    let file = |path: String| WatchSubject {
        kind: SubscriptionKind::File,
        path,
        suffix: None,
    };
    let mut frontier: Vec<Node> = SKILLS_DIRECTORIES
        .iter()
        .enumerate()
        .map(|(rank, skills)| Node {
            path: join(directory, skills),
            rank,
            depth: 0,
        })
        .collect();
    let mut watch: Vec<WatchSubject> = Vec::new();
    let mut found: Vec<(usize, ProjectSkill)> = Vec::new();
    let mut read = 0;
    while !frontier.is_empty() && read < MAX_DIRECTORIES && found.len() < MAX_SKILLS {
        let room = MAX_DIRECTORIES - read;
        frontier.truncate(room);
        read += frontier.len();
        let reads: Vec<HostRead> = frontier
            .iter()
            .flat_map(|node| {
                // A skills directory has no manifest; the folder it is in is
                // read instead, to know which of the two is missing.
                let second = if node.depth == 0 {
                    HostRead::new(above(&node.path), 0)
                } else {
                    HostRead::new(
                        join(&node.path, "SKILL.md"),
                        u64::try_from(SKILL_MD_MAX_BYTES).expect("a size fits") + 1,
                    )
                };
                [HostRead::new(node.path.clone(), 0), second]
            })
            .collect();
        let files = port.read_host_files(reads).await?;
        let mut next = Vec::new();
        for (node, answers) in frontier.iter().zip(files.chunks(2)) {
            let [listing, second] = answers else {
                unreachable!("two answers per node")
            };
            let entries = match listing {
                HostFile::Directory { entries } => Some(entries),
                _ => None,
            };
            if node.depth == 0 {
                if entries.is_none() {
                    let missing = match second {
                        HostFile::Directory { .. } => node.path.clone(),
                        _ => above(&node.path),
                    };
                    watch.push(file(missing));
                }
            } else {
                watch.push(file(join(&node.path, "SKILL.md")));
                if let HostFile::File { bytes, size } = second {
                    if let Some(skill) = project_skill(paths, &node.path, bytes.as_bytes(), *size) {
                        found.push((node.rank, skill));
                    }
                    continue;
                }
            }
            let Some(entries) = entries else {
                continue;
            };
            watch.push(WatchSubject {
                kind: SubscriptionKind::FolderNames,
                path: node.path.clone(),
                suffix: None,
            });
            if node.depth == MAX_DEPTH {
                continue;
            }
            for entry in entries {
                if matches!(entry.kind, EntryKind::Directory | EntryKind::Symlink) {
                    next.push(Node {
                        path: join(&node.path, &entry.name),
                        rank: node.rank,
                        depth: node.depth + 1,
                    });
                }
            }
        }
        frontier = next;
    }
    found.truncate(MAX_SKILLS);
    // Stable: in one skills directory, the search's order holds.
    found.sort_by_key(|(rank, _)| *rank);
    let mut skills: Vec<ProjectSkill> = Vec::new();
    for (_, skill) in found {
        match skills.iter().find(|kept| kept.name == skill.name) {
            Some(kept) => tracing::info!(
                skill = %skill.location,
                kept = %kept.location,
                "a project skill is shadowed by one of the same name"
            ),
            None => skills.push(skill),
        }
    }
    Ok((skills, watch))
}

/// A directory the search reads.
struct Node {
    path: String,
    /// The precedence of the skills directory it is in, the first first.
    rank: usize,
    depth: usize,
}

/// The skill in `directory` whose `SKILL.md` is `text`, of `size` bytes;
/// none, logged, when it is not one.
fn project_skill(paths: HostPaths, directory: &str, text: &[u8], size: u64) -> Option<ProjectSkill> {
    let location = paths.join(directory, "SKILL.md");
    let name = paths.file_name(directory);
    let too_large = usize::try_from(size).map_or(true, |size| size > SKILL_MD_MAX_BYTES);
    let parsed = if too_large {
        Err(skill::NotASkill::too_large())
    } else {
        skill::parse(name, text)
    };
    match parsed {
        Ok(parsed) => {
            for warning in &parsed.warnings {
                tracing::info!(skill = %location, %warning, "a project skill has a warning");
            }
            Some(ProjectSkill {
                name: parsed.name,
                description: parsed.description,
                location,
                disable_model_invocation: parsed.disable_model_invocation,
            })
        }
        Err(reason) => {
            tracing::info!(skill = %location, %reason, "a project SKILL.md is not a skill");
            None
        }
    }
}
