//! Project skills (`skills.md` § Project skills): the skills a repository
//! carries in `.agents/skills` and `.claude/skills`, in the node's working
//! directory and each directory above it up to the repository's root,
//! found through Host reads that never wake the Host. Each round of the
//! search reads every path it needs in one request.

use demi_plugin_interface::project::{self, join};
use demi_plugin_interface::{EntryKind, HostFile, HostRead, PluginPort, PortFailure};

use crate::skill::{self, SKILL_MD_MAX_BYTES};

/// The most directories one search reads.
const MAX_DIRECTORIES: usize = 2_000;

/// How deep below a skills directory a search looks.
const MAX_DEPTH: usize = 6;

/// The most project skills one search finds.
const MAX_SKILLS: usize = 100;

/// The skills directories of each directory searched, nearer first.
const SKILLS_DIRECTORIES: [&str; 2] = [".agents/skills", ".claude/skills"];

/// A project skill, listed at its own path on the Host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectSkill {
    pub(crate) name: String,
    pub(crate) description: String,
    /// The path of its `SKILL.md` on the Host.
    pub(crate) location: String,
    pub(crate) disable_model_invocation: bool,
}

/// The project skills for a node working in `cwd`, one per name: of two of
/// a name, the one in the nearer directory, and in one directory,
/// `.agents/skills` over `.claude/skills`. What is not a skill, has
/// warnings or is shadowed is logged.
pub(crate) async fn search(port: &PluginPort, cwd: &str) -> Result<Vec<ProjectSkill>, PortFailure> {
    let searched = searched_directories(port, cwd).await?;
    let mut frontier: Vec<Node> = Vec::new();
    for (rank, directory) in searched.iter().enumerate() {
        for (place, skills) in SKILLS_DIRECTORIES.iter().enumerate() {
            frontier.push(Node {
                path: join(directory, skills),
                rank: rank * SKILLS_DIRECTORIES.len() + place,
                depth: 0,
            });
        }
    }
    let mut found: Vec<(usize, ProjectSkill)> = Vec::new();
    let mut read = 0;
    while !frontier.is_empty() && read < MAX_DIRECTORIES && found.len() < MAX_SKILLS {
        let room = MAX_DIRECTORIES - read;
        frontier.truncate(room);
        read += frontier.len();
        let reads: Vec<HostRead> = frontier
            .iter()
            .flat_map(|node| {
                [
                    HostRead {
                        path: node.path.clone(),
                        limit: 0,
                    },
                    HostRead {
                        path: join(&node.path, "SKILL.md"),
                        limit: u64::try_from(SKILL_MD_MAX_BYTES).expect("a size fits") + 1,
                    },
                ]
            })
            .collect();
        let files = port.read_host_files(reads).await?;
        let mut next = Vec::new();
        for (node, answers) in frontier.iter().zip(files.chunks(2)) {
            let [listing, manifest] = answers else {
                unreachable!("two answers per node")
            };
            if node.depth > 0
                && let HostFile::File { bytes, size } = manifest
            {
                if let Some(skill) = project_skill(&node.path, bytes.as_bytes(), *size) {
                    found.push((node.rank, skill));
                }
                continue;
            }
            let HostFile::Directory { entries } = listing else {
                continue;
            };
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
    Ok(skills)
}

/// A directory the search reads.
struct Node {
    path: String,
    /// The precedence of the skills directory it is in, nearer first.
    rank: usize,
    depth: usize,
}

/// `cwd` and each directory above it up to the root of its git repository.
async fn searched_directories(port: &PluginPort, cwd: &str) -> Result<Vec<String>, PortFailure> {
    let ancestors = project::ancestors(cwd);
    let found = port.read_host_files(project::root_reads(&ancestors)).await?;
    Ok(project::searched(&ancestors, &found).to_vec())
}

/// The skill in `directory` whose `SKILL.md` is `text`, of `size` bytes;
/// none, logged, when it is not one.
fn project_skill(directory: &str, text: &[u8], size: u64) -> Option<ProjectSkill> {
    let location = join(directory, "SKILL.md");
    let name = directory.rsplit('/').next().unwrap_or(directory);
    let too_large = usize::try_from(size).map_or(true, |size| size > SKILL_MD_MAX_BYTES);
    let parsed = if too_large {
        Err("SKILL.md is larger than 256 KiB".to_owned())
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
