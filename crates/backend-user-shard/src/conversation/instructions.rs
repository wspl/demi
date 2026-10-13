//! The instructions source (`instructions.md`): the user's personal
//! instructions and the project's `AGENTS.md` or `CLAUDE.md` file and
//! `.demi/instructions.md` of each directory from the repository's root
//! down to the node's working directory, and for the root the instructions
//! of the workflow the conversation chose (`subagents.md` § Workflows of a
//! project), as one context block, with the list of what it holds for the
//! page.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Weak;

use demi_agent_tools::{ContextAnswer, ContextSource, NodeContext};
use demi_backend_host_access::conversation_of;
use demi_backend_host_access::plugin_files::ReadFilesError;
use demi_plugin_interface::project;
use demi_plugin_interface::{HostFile, HostRead};
use demi_shared_types::{INSTRUCTIONS_SOURCE, InstructionEntry, TurnId, WorkflowChoice, xml_escaped};
use demi_web_api_protocol::ids::ConversationId;
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

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
struct ProjectFile {
    path: String,
    text: String,
}

/// The last search for a conversation and a working directory, and the
/// input turn it ran in.
struct Search {
    turn: TurnId,
    files: Vec<ProjectFile>,
}

/// A workflow's name and instructions, which the root reads while the
/// conversation uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkflowText {
    name: String,
    instructions: String,
}

/// The last read of a conversation's workflows and the input turn it ran
/// in: each workflow that reads and has instructions, by id.
struct WorkflowRead {
    turn: TurnId,
    texts: Vec<(String, WorkflowText)>,
}

/// The product's instructions source, asked after the execution context.
pub(crate) struct Instructions {
    pub(crate) shard: Weak<Shard>,
    searches: RefCell<HashMap<(ConversationId, String), Search>>,
    workflows: RefCell<HashMap<ConversationId, WorkflowRead>>,
}

impl Instructions {
    pub(crate) fn new(shard: Weak<Shard>) -> Self {
        Self {
            shard,
            searches: RefCell::default(),
            workflows: RefCell::default(),
        }
    }

    /// The instructions of the workflow `id` the conversation chose, none
    /// when it has none or does not read: the workflows are read at the
    /// first request of each input turn that needs them, as the project
    /// files are, and what the last read found is used while a read cannot
    /// run.
    async fn workflow_text(
        &self,
        shard: &Shard,
        conversation: &ConversationId,
        id: &str,
        turn: &TurnId,
    ) -> Option<WorkflowText> {
        let current = self
            .workflows
            .borrow()
            .get(conversation)
            .is_some_and(|read| read.turn == *turn);
        if !current {
            // The request ends with the provider request it serves, which
            // drops this future.
            match shard.project_workflows(conversation, &CancellationToken::new()).await {
                Ok(found) => {
                    let texts = found
                        .files
                        .into_iter()
                        .filter_map(|file| {
                            let workflow = file.read.ok()?;
                            let instructions = workflow.instructions.filter(|text| !text.trim().is_empty())?;
                            Some((file.id, WorkflowText { name: workflow.name, instructions }))
                        })
                        .collect();
                    let read = WorkflowRead {
                        turn: turn.clone(),
                        texts,
                    };
                    self.workflows.borrow_mut().insert(conversation.clone(), read);
                }
                Err(error) => {
                    if !matches!(error, ReadFilesError::NotRunning) {
                        tracing::warn!(%conversation, %error, "the project's workflows were not read");
                    }
                }
            }
        }
        self.workflows
            .borrow()
            .get(conversation)?
            .texts
            .iter()
            .find(|(listed, _)| listed == id)
            .map(|(_, text)| text.clone())
    }

    /// The project files for a node in `cwd`: searched at the first request
    /// of each input turn, and again at the next request of the turn while
    /// the Host was not running; what the last search found otherwise.
    async fn project_files(
        &self,
        shard: &Shard,
        conversation: ConversationId,
        cwd: &str,
        turn: &TurnId,
    ) -> Vec<ProjectFile> {
        let key = (conversation, cwd.to_owned());
        if let Some(search) = self.searches.borrow().get(&key)
            && search.turn == *turn
        {
            return search.files.clone();
        }
        // The request ends with the provider request it serves, which drops
        // this future.
        match search(shard, &key.0, cwd, &CancellationToken::new()).await {
            Ok(files) => {
                let search = Search {
                    turn: turn.clone(),
                    files: files.clone(),
                };
                self.searches.borrow_mut().insert(key, search);
                files
            }
            Err(error) => {
                if !matches!(error, ReadFilesError::NotRunning) {
                    tracing::warn!(%cwd, %error, "the project's instruction files were not searched");
                }
                self.searches
                    .borrow()
                    .get(&key)
                    .map(|search| search.files.clone())
                    .unwrap_or_default()
            }
        }
    }
}

impl ContextSource for Instructions {
    fn name(&self) -> &str {
        INSTRUCTIONS_SOURCE
    }

    fn context<'a>(
        &'a self,
        node: NodeContext<'a>,
        turn: &'a TurnId,
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
            let files = self
                .project_files(&shard, conversation.clone(), &cwd, turn)
                .await;
            // The workflow's instructions are the root's alone.
            let workflow = if node.node == node.root {
                match shard.workflow_choice(&conversation).await? {
                    WorkflowChoice::Workflow { id } => {
                        self.workflow_text(&shard, &conversation, &id, turn).await
                    }
                    WorkflowChoice::Default | WorkflowChoice::None => None,
                }
            } else {
                None
            };
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

/// The project files for a node in `cwd`, root first, whole: one request
/// finds `.git` and each name in every directory from `cwd` up with their
/// sizes, and one more reads the files taken at those sizes; a file that
/// grew meanwhile is read again.
async fn search(
    shard: &Shard,
    conversation: &ConversationId,
    cwd: &str,
    cancel: &CancellationToken,
) -> Result<Vec<ProjectFile>, ReadFilesError> {
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
            });
        }
        reads.push(HostRead {
            path: demi_file(directory),
            limit: 0,
        });
    }
    let (found, _) = shard
        .host_shard()
        .read_files(conversation, &reads, cancel)
        .await?;
    let (git, named) = found.split_at(ancestors.len());
    let searched = project::searched(&ancestors, git).len();
    let taken_file = |path: String, answer: &HostFile| match answer {
        HostFile::File { size, .. } => Some(HostRead { path, limit: *size }),
        HostFile::Unreadable { message } => {
            tracing::info!(%path, %message, "a project instruction file cannot be read");
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
        let (answers, _) = shard
            .host_shard()
            .read_files(conversation, &reads, cancel)
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
            tracing::info!(path = %read.path, "a project instruction file kept growing while it was read; it is left out until the next turn");
            continue;
        };
        if !text.trim().is_empty() {
            files.push(ProjectFile { path: read.path, text });
        }
    }
    Ok(files)
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
    for (entry, body) in personal.into_iter().chain(files) {
        text.push_str("\n\n");
        text.push_str(&entry.section(body));
        instructions.push(entry);
    }
    if let Some(workflow) = workflow {
        text.push_str(&format!(
            "\n\n<workflow name=\"{}\">\n{}\n</workflow>",
            xml_escaped(&workflow.name),
            workflow.instructions.trim_end()
        ));
    }
    Some(ContextAnswer { text, instructions })
}
