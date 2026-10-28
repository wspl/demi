//! The instructions every node reads: the user's personal instructions, the
//! project's instruction files and a workflow's instructions, as one context
//! block.
//!
//! The project's files are the `AGENTS.md` or `CLAUDE.md` file and
//! `.demi/instructions.md` of each directory from the repository's root down
//! to the node's working directory ([`search`]). A product reads them through
//! the reader it passes, keeps them, and renders the block for its context
//! source ([`render`]).
//!
//! # Design choices
//!
//! - The text is a context block, not part of the system prompt. The system
//!   prompt stays the same for every user and project so that its prefix is
//!   cached; an edit, or a random order, would cost the cache.
//! - One file is read per directory, the first name that exists, so a
//!   `CLAUDE.md` that links to or copies the `AGENTS.md` beside it reaches the
//!   model once.
//! - The git root bounds the search, as walking to the file system's root would
//!   read unrelated files above the project.
//! - Home directories are not read: `~/.claude/CLAUDE.md` would differ between
//!   the Hosts a conversation moves to, and instructions for every project are
//!   the personal instructions, which follow the user.
//! - Nothing is cut. Dropping the end of a project's rules without telling
//!   anyone is worse than a large request the context usage shows, and a file
//!   left out for its size is the same loss.

use std::future::Future;

use demi_agent_tools::ContextAnswer;
use demi_host_interface::project;
use demi_host_interface::{HostFile, HostPaths, HostRead};
use demi_shared_types::InstructionEntry;

/// The names read in each directory, the first that is a file taken.
const NAMES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// Demi's own instructions of each directory, read after the one of
/// [`NAMES`] it has: the directory and the file's name.
const DEMI_FILE: [&str; 2] = [".demi", "instructions.md"];

const PREAMBLE: &str = "These are the user's personal instructions and the project's instruction files. Follow them in this conversation together with your other instructions; this block replaces any earlier one. Where they conflict, the user's messages come first, then a file nearer the working directory over one further up, then the personal instructions.";

/// What a source answers once the newest block held something and there is
/// nothing now.
pub const NONE_LEFT: &str = "There are no longer any personal instructions or project instruction files.";

/// How many times a file that grew while it was read is read again.
///
/// Bounded so a file that keeps growing ends the read instead of repeating it
/// without end.
const REREADS: usize = 3;

/// A project file the search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFile {
    pub path: String,
    pub text: String,
}

/// A workflow's name, file, and its instructions with its profiles, which
/// the root reads while the conversation uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowText {
    pub name: String,
    pub path: String,
    pub text: String,
}

/// What a [`search`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    /// The files, root first, whole; an empty one left out.
    pub files: Vec<ProjectFile>,
    /// The files to watch: each one read or that would be read, and the
    /// `.git` whose coming moves the root nearer.
    pub watch: Vec<String>,
    /// The files the Host could not read, each with its reason.
    pub unreadable: Vec<(String, String)>,
    /// The files left out because they kept growing while they were read.
    pub growing: Vec<String>,
}

/// The project files for a node in `cwd` on a Host whose paths are `paths`,
/// read through `read`.
///
/// It reads in two requests: one finds `.git` and each name in every directory
/// from `cwd` up with their sizes, and one more reads the files taken at those
/// sizes. A file that grew meanwhile is read again.
///
/// It names to watch each file it read or would read: in each directory
/// searched its names, and the `.git` whose coming moves the root nearer.
/// Outside a repository, the `.git` of every directory above. The root's own
/// `.git` is left out: git writes in it at every command.
pub async fn search<E, R, F>(paths: HostPaths, cwd: &str, mut read: R) -> Result<Search, E>
where
    R: FnMut(Vec<HostRead>) -> F,
    F: Future<Output = Result<Vec<HostFile>, E>>,
{
    let join = |directory: &str, name: &str| paths.join(directory, name);
    let ancestors = paths.ancestors(cwd);
    let [demi, demi_name] = DEMI_FILE;
    let demi_file = |directory: &str| join(&join(directory, demi), demi_name);
    let mut reads = project::root_reads(paths, &ancestors);
    for directory in &ancestors {
        for name in NAMES {
            reads.push(HostRead::new(join(directory, name), 0));
        }
        reads.push(HostRead::new(demi_file(directory), 0));
    }
    let found = read(reads).await?;
    let (git, named) = found.split_at(ancestors.len());
    let searched = project::searched(&ancestors, git).len();
    let mut watch = project::root_watch(paths, &ancestors, git);
    for directory in &ancestors[..searched] {
        watch.extend(NAMES.iter().map(|name| join(directory, name)));
        watch.push(demi_file(directory));
    }
    let mut unreadable = Vec::new();
    let mut taken_file = |path: String, answer: &HostFile| match answer {
        HostFile::File { size, .. } => Some(HostRead::new(path, *size)),
        HostFile::Unreadable { message } => {
            unreadable.push((path, message.clone()));
            None
        }
        HostFile::Missing | HostFile::Directory { .. } | HostFile::Other => None,
    };
    // Each directory's files in their order, nearest directory first.
    let mut by_directory: Vec<Vec<HostRead>> = Vec::new();
    for (directory, answers) in ancestors.iter().zip(named.chunks(NAMES.len() + 1)).take(searched) {
        let (shared, own) = answers.split_at(NAMES.len());
        let file = NAMES
            .iter()
            .zip(shared)
            .find_map(|(name, answer)| taken_file(join(directory, name), answer));
        let demi = taken_file(demi_file(directory), &own[0]);
        by_directory.push(file.into_iter().chain(demi).collect());
    }
    let mut taken: Vec<HostRead> = by_directory.into_iter().rev().flatten().collect();
    let mut texts: Vec<Option<String>> = vec![None; taken.len()];
    for _ in 0..=REREADS {
        let pending: Vec<usize> = (0..taken.len()).filter(|&index| texts[index].is_none()).collect();
        if pending.is_empty() {
            break;
        }
        let reads: Vec<HostRead> = pending.iter().map(|&index| taken[index].clone()).collect();
        let answers = read(reads).await?;
        for (index, answer) in pending.into_iter().zip(answers) {
            match answer {
                HostFile::File { bytes, size } if size <= taken[index].limit => {
                    texts[index] = Some(String::from_utf8_lossy(bytes.as_bytes()).into_owned());
                }
                // It grew since its size was read: read it again at its new size.
                HostFile::File { size, .. } => taken[index].limit = size,
                // Gone or changed into something else since: it is left out.
                _ => texts[index] = Some(String::new()),
            }
        }
    }
    let mut files = Vec::new();
    let mut growing = Vec::new();
    for (read, text) in taken.into_iter().zip(texts) {
        let Some(text) = text else {
            growing.push(read.path);
            continue;
        };
        if !text.trim().is_empty() {
            files.push(ProjectFile { path: read.path, text });
        }
    }
    Ok(Search {
        files,
        watch,
        unreadable,
        growing,
    })
}

/// The block for `personal`, `files` and the chosen `workflow`, which
/// joins it as an element of its own after the project's files; none when
/// there is nothing.
pub fn render(personal: &str, files: &[ProjectFile], workflow: Option<&WorkflowText>) -> Option<ContextAnswer> {
    if personal.is_empty() && files.is_empty() && workflow.is_none() {
        return None;
    }
    let mut text = String::from(PREAMBLE);
    let mut instructions = Vec::new();
    let personal = (!personal.is_empty()).then(|| (InstructionEntry::Personal, personal));
    let files = files.iter().map(|file| {
        let entry = InstructionEntry::File {
            path: file.path.clone(),
        };
        (entry, file.text.as_str())
    });
    let workflow = workflow.map(|workflow| {
        let entry = InstructionEntry::Workflow {
            name: workflow.name.clone(),
            path: workflow.path.clone(),
        };
        (entry, workflow.text.as_str())
    });
    for (entry, body) in personal.into_iter().chain(files).chain(workflow) {
        text.push_str("\n\n");
        text.push_str(&entry.section(body));
        instructions.push(entry);
    }
    Some(ContextAnswer { text, instructions })
}
