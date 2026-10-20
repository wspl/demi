//! The instructions source.
//!
//! It builds one context block, with the list of what it holds for the page,
//! from:
//!
//! - the user's personal instructions;
//! - the project's `AGENTS.md` or `CLAUDE.md` file and `.demi/instructions.md`
//!   of each directory from the repository's root down to the node's working
//!   directory;
//! - for the root, the instructions of the workflow the conversation chose
//!   (`crates/backend-user-shard/src/conversation/workflows.rs`).
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
//!   the Cloud and each device the conversation moves to, and instructions for
//!   every project are the personal instructions, which follow the user.
//! - Nothing is cut. Dropping the end of a project's rules without telling
//!   anyone is worse than a large request the context usage shows, and a file
//!   left out for its size is the same loss.

use std::rc::{Rc, Weak};

use demi_agent_tools::{ContextAnswer, ContextSource, NodeContext};
use demi_backend_host_access::conversation_of;
use demi_backend_host_access::kept::Found;
use demi_backend_host_access::plugin_files::ReadFilesError;
use demi_runner_protocol::files::{Subject, SubscriptionKind};
use demi_plugin_interface::project;
use demi_plugin_interface::{HostFile, HostRead};
use demi_shared_types::{INSTRUCTIONS_SOURCE, InstructionEntry, TurnId, WorkflowChoice};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use super::project_files::KeptWorkflows;
use crate::shard::Shard;

/// The names read in each directory, the first that is a file taken.
const NAMES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// Demi's own instructions of each directory, read after the one of
/// [`NAMES`] it has: the directory and the file's name.
const DEMI_FILE: [&str; 2] = [".demi", "instructions.md"];

const PREAMBLE: &str = "These are the user's personal instructions and the project's instruction files. Follow them in this conversation together with your other instructions; this block replaces any earlier one. Where they conflict, the user's messages come first, then a file nearer the working directory over one further up, then the personal instructions.";

/// What the source answers once the newest block held something and there
/// is nothing now.
const NONE_LEFT: &str = "There are no longer any personal instructions or project instruction files.";

/// A project file the search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectFile {
    path: String,
    text: String,
}

/// A workflow's name, file and instructions, which the root reads while
/// the conversation uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkflowText {
    name: String,
    path: String,
    instructions: String,
}

/// The product's instructions source, asked after the execution context.
pub(crate) struct Instructions {
    pub(crate) shard: Weak<Shard>,
}

impl Instructions {
    pub(crate) fn new(shard: Weak<Shard>) -> Self {
        Self { shard }
    }
}

/// The instructions of the workflow `id` the conversation chose, as the
/// shard holds its workflows; none when it has none or cannot be used.
async fn workflow_text(shard: &Shard, conversation: &ConversationId, id: &str) -> Option<WorkflowText> {
    let found = match shard.kept_workflows(conversation).await {
        Ok(KeptWorkflows::Found(found)) => found,
        Ok(KeptWorkflows::None) => return None,
        Err(error) => {
            tracing::warn!(%conversation, %error, "the project's workflows were not taken");
            return None;
        }
    };
    if found.unavailable.is_some() {
        return None;
    }
    let file = found.files.iter().find(|file| file.id == id)?;
    let workflow = file.read.as_ref().ok()?;
    let instructions = workflow.instructions.clone().filter(|text| !text.trim().is_empty())?;
    Some(WorkflowText {
        name: workflow.name.clone(),
        path: file.path.clone(),
        instructions,
    })
}

impl ContextSource for Instructions {
    fn name(&self) -> &str {
        INSTRUCTIONS_SOURCE
    }

    fn context<'a>(
        &'a self,
        node: NodeContext<'a>,
        _: &'a TurnId,
        seen: &'a [&'a str],
    ) -> LocalBoxFuture<'a, Result<Option<ContextAnswer>, String>> {
        Box::pin(async move {
            let Some(shard) = self.shard.upgrade() else {
                return Ok(None);
            };
            let personal = shard
                .services()
                .control
                .instructions(shard.user().clone())
                .await
                .map_err(|error| format!("the personal instructions cannot be read: {error}"))?;
            let cwd = shard
                .node_directory(node)
                .await
                .map_err(|error| format!("the working directory cannot be read: {error}"))?;
            let conversation = conversation_of(node.root);
            // The workflow's instructions are the root's alone. Its
            // discovery and the files' start together, so their first
            // searches share one hold.
            let chosen = if node.node == node.root {
                match shard.workflow_choice(&conversation).await? {
                    WorkflowChoice::Workflow { id } => Some(id),
                    WorkflowChoice::Default | WorkflowChoice::None => None,
                }
            } else {
                None
            };
            let workflow = async {
                match &chosen {
                    Some(id) => workflow_text(&shard, &conversation, id).await,
                    None => None,
                }
            };
            let (files, workflow) = futures_util::join!(shard.kept_instructions(&conversation, &cwd), workflow);
            let answer = match render(&personal, &files, workflow.as_ref()) {
                Some(answer) => answer,
                None if seen.last().is_some_and(|newest| *newest != NONE_LEFT) => {
                    ContextAnswer::from(NONE_LEFT.to_owned())
                }
                None => return Ok(None),
            };
            Ok((seen.last() != Some(&answer.text.as_str())).then_some(answer))
        })
    }
}

/// How many times a file that grew while it was read is read again.
const REREADS: usize = 3;

/// The project files for a node in `cwd`, root first, whole.
///
/// It reads in two requests: one finds `.git` and each name in every directory
/// from `cwd` up with their sizes, and one more reads the files taken at those
/// sizes. A file that grew meanwhile is read again.
///
/// It watches each file it read or would read: in each directory searched its
/// names, and the `.git` whose coming moves the root nearer. Outside a
/// repository, it watches the `.git` of every directory above. The root's own
/// `.git` is left out: git writes in it at every command.
pub(crate) async fn search_instructions(
    shard: &Shard,
    conversation: &ConversationId,
    device: &DeviceId,
    cwd: &str,
    cancel: &CancellationToken,
) -> Result<Found<Rc<[ProjectFile]>>, ReadFilesError> {
    let paths = shard
        .host_paths(conversation)
        .await
        .map_err(|error| ReadFilesError::Access(error.into()))?;
    let join = |directory: &str, name: &str| paths.join(directory, name);
    let ancestors = paths.ancestors(cwd);
    let [demi, demi_name] = DEMI_FILE;
    let demi_file = |directory: &str| join(&join(directory, demi), demi_name);
    let mut reads = project::root_reads(paths, &ancestors);
    for directory in &ancestors {
        for name in NAMES {
            reads.push(HostRead {
                path: join(directory, name),
                limit: 0,
    within: None,
});
        }
        reads.push(HostRead::new(demi_file(directory), 0));
    }
    let (found, _) = shard
        .host_shard()
        .read_files_on(conversation, Some(device), &reads, cancel)
        .await?;
    let (git, named) = found.split_at(ancestors.len());
    let searched = project::searched(&ancestors, git).len();
    let file = |path: String| Subject {
        kind: SubscriptionKind::File,
        path,
        suffix: None,
    };
    let mut watch: Vec<Subject> = project::root_watch(paths, &ancestors, git).into_iter().map(file).collect();
    for directory in &ancestors[..searched] {
        watch.extend(NAMES.iter().map(|name| file(join(directory, name))));
        watch.push(file(demi_file(directory)));
    }
    let taken_file = |path: String, answer: &HostFile| match answer {
        HostFile::File { size, .. } => Some(HostRead { path, limit: *size, within: None }),
        HostFile::Unreadable { message } => {
            tracing::info!(%path, %message, "a project instruction file cannot be read");
            None
        }
        // Instruction files are read through links: none answers a link.
        HostFile::Missing | HostFile::Directory { .. } | HostFile::Other | HostFile::Link => None,
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
        let (answers, _) = shard
            .host_shard()
            .read_files_on(conversation, Some(device), &reads, cancel)
            .await?;
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
    for (read, text) in taken.into_iter().zip(texts) {
        let Some(text) = text else {
            tracing::info!(path = %read.path, "a project instruction file kept growing while it was read; it is left out until it next changes");
            continue;
        };
        if !text.trim().is_empty() {
            files.push(ProjectFile { path: read.path, text });
        }
    }
    Ok(Found {
        value: files.into(),
        watch,
    })
}

/// The block for `personal`, `files` and the chosen `workflow`'s
/// instructions, which join it as an element of their own after the
/// project's files; none when all are empty.
fn render(
    personal: &str,
    files: &[ProjectFile],
    workflow: Option<&WorkflowText>,
) -> Option<ContextAnswer> {
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
        (entry, workflow.instructions.trim_end())
    });
    for (entry, body) in personal.into_iter().chain(files).chain(workflow) {
        text.push_str("\n\n");
        text.push_str(&entry.section(body));
        instructions.push(entry);
    }
    Some(ContextAnswer { text, instructions })
}
