//! The instructions source (`instructions.md`): the user's personal
//! instructions and the project's `AGENTS.md` or `CLAUDE.md` files, one per
//! directory from the repository's root down to the node's working
//! directory, as one context block, with the list of what it holds for the
//! page.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Weak;

use demi_agent_tools::{ContextAnswer, ContextSource, NodeContext};
use demi_backend_host_access::conversation_of;
use demi_backend_host_access::plugin_files::ReadFilesError;
use demi_plugin_interface::project::{self, join};
use demi_plugin_interface::{HostFile, HostRead};
use demi_shared_types::{INSTRUCTIONS_SOURCE, InstructionEntry, TurnId, xml_escaped};
use demi_web_api_protocol::ids::ConversationId;
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::shard::Shard;

/// The names read in each directory, the first that is a file taken.
const NAMES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// The largest file sent; a larger one is named, not read.
const FILE_MAX_BYTES: u64 = 256 * 1024;

const PREAMBLE: &str = "These are the user's personal instructions and the project's instruction files. Follow them in this conversation together with your other instructions; this block replaces any earlier one. Where they conflict, the user's messages come first, then a file nearer the working directory over one further up, then the personal instructions.";

/// What the source answers once the newest block held something and there
/// is nothing now.
const NONE_LEFT: &str = "There are no longer any personal instructions or project instruction files.";

/// A project file the search found.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProjectFile {
    Text { path: String, text: String },
    TooLarge { path: String },
}

/// The last search for a conversation and a working directory, and the
/// input turn it ran in.
struct Search {
    turn: TurnId,
    files: Vec<ProjectFile>,
}

/// The product's instructions source, asked after the execution context.
pub(crate) struct Instructions {
    pub(crate) shard: Weak<Shard>,
    searches: RefCell<HashMap<(ConversationId, String), Search>>,
}

impl Instructions {
    pub(crate) fn new(shard: Weak<Shard>) -> Self {
        Self {
            shard,
            searches: RefCell::default(),
        }
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
            let files = self
                .project_files(&shard, conversation_of(node.root), node.cwd, turn)
                .await;
            let answer = match render(&personal, &files) {
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

/// The project files for a node in `cwd`, root first, read with one
/// request: `.git` and each name in every directory from `cwd` up.
async fn search(
    shard: &Shard,
    conversation: &ConversationId,
    cwd: &str,
    cancel: &CancellationToken,
) -> Result<Vec<ProjectFile>, ReadFilesError> {
    let ancestors = project::ancestors(cwd);
    let mut reads = project::root_reads(&ancestors);
    for directory in &ancestors {
        for name in NAMES {
            reads.push(HostRead {
                path: join(directory, name),
                limit: FILE_MAX_BYTES + 1,
            });
        }
    }
    let found = shard
        .host_shard()
        .read_files(conversation, &reads, cancel)
        .await?;
    let (git, named) = found.split_at(ancestors.len());
    let searched = project::searched(&ancestors, git).len();
    let mut files = Vec::new();
    for (directory, answers) in ancestors.iter().zip(named.chunks(NAMES.len())).take(searched) {
        let taken = NAMES.iter().zip(answers).find_map(|(name, answer)| {
            let path = join(directory, name);
            match answer {
                HostFile::File { bytes, size } => Some(project_file(path, bytes.as_bytes(), *size)),
                HostFile::Unreadable { message } => {
                    tracing::info!(%path, %message, "a project instruction file cannot be read");
                    None
                }
                HostFile::Missing | HostFile::Directory { .. } | HostFile::Other => None,
            }
        });
        files.extend(taken.flatten());
    }
    files.reverse();
    Ok(files)
}

/// The file at `path` whose first bytes are `bytes`, of `size` bytes; none
/// when it holds only white space.
fn project_file(path: String, bytes: &[u8], size: u64) -> Option<ProjectFile> {
    if size > FILE_MAX_BYTES {
        return Some(ProjectFile::TooLarge { path });
    }
    let text = String::from_utf8_lossy(bytes).into_owned();
    (!text.trim().is_empty()).then_some(ProjectFile::Text { path, text })
}

/// The block for `personal` and `files`; none when both are empty.
fn render(personal: &str, files: &[ProjectFile]) -> Option<ContextAnswer> {
    if personal.is_empty() && files.is_empty() {
        return None;
    }
    let mut text = String::from(PREAMBLE);
    let mut instructions = Vec::new();
    if !personal.is_empty() {
        text.push_str(&format!(
            "\n\n<personal_instructions>\n{personal}\n</personal_instructions>"
        ));
        instructions.push(InstructionEntry::Personal);
    }
    for file in files {
        match file {
            ProjectFile::Text { path, text: file } => {
                text.push_str(&format!(
                    "\n\n<project_instructions path=\"{}\">\n{file}\n</project_instructions>",
                    xml_escaped(path)
                ));
                instructions.push(InstructionEntry::File { path: path.clone() });
            }
            ProjectFile::TooLarge { path } => {
                text.push_str(&format!(
                    "\n\n<project_instructions path=\"{}\">\nThis file is larger than 256 KiB and is not included. Read the parts the task needs with your commands.\n</project_instructions>",
                    xml_escaped(path)
                ));
                instructions.push(InstructionEntry::TooLarge { path: path.clone() });
            }
        }
    }
    Some(ContextAnswer { text, instructions })
}
