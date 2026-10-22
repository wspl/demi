//! The attachments a transcript's replies name, which travel beside the
//! blocks as failure facts do, never stored with them
//! (`docs/decisions/0029-a-conversation-opens-whole.md`).
//!
//! A reply names an attachment by a line that is its tag alone,
//! `<attachment id="a3" />`, the rule the page's renderer follows, and by the
//! copy it keeps of a picture of the Host it shows
//! (`demi_backend_database::shares::block_attachments`). The socket's
//! transcript frames and the history route both carry the attachments their
//! blocks and their text name, whoever in the conversation stored them, so the
//! page draws each card or picture at its size with the reply, and asks for
//! none.
//!
//! A reply streams as appended text, and a tag may arrive in pieces, so a
//! frame that completes one is named from the block's text so far, never from
//! its own delta: [`StreamedText`] keeps what a socket needs of it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use demi_backend_database::StorageError;
use demi_backend_database::conversation_attachments::{AttachmentNumber, read_all};
use demi_backend_database::conversations::ConversationStores;
use demi_backend_database::shares::{block_attachments, named_attachments};
use demi_conversation_socket_protocol::{ConversationAttachment, TranscriptPatch};
use demi_shared_types::{Block, NodeId, TextBlock};
use demi_web_api_protocol::ids::ConversationId;

/// How many text blocks of a node a socket keeps the text of: the one being
/// written and the few before it.
const KEPT_BLOCKS: usize = 8;

/// The attachments of `conversation` named by `replies`, the replies' text
/// blocks, in their order; none for a conversation without a database yet.
pub async fn attachment_facts<'a>(
    conversations: &ConversationStores,
    conversation: &ConversationId,
    replies: impl IntoIterator<Item = &'a TextBlock>,
) -> Result<Vec<ConversationAttachment>, StorageError> {
    described(conversations, conversation, replies.into_iter().flat_map(block_attachments).collect()).await
}

/// The descriptions of `numbers` that the conversation has.
async fn described(
    conversations: &ConversationStores,
    conversation: &ConversationId,
    numbers: BTreeSet<AttachmentNumber>,
) -> Result<Vec<ConversationAttachment>, StorageError> {
    if numbers.is_empty() {
        return Ok(Vec::new());
    }
    let rows = conversations
        .read(conversation, move |connection| read_all(connection, &numbers))
        .await?
        .unwrap_or_default();
    Ok(rows.into_iter().map(|row| row.description()).collect())
}

/// The replies among `blocks`.
pub fn reply_blocks(blocks: &[Block]) -> impl Iterator<Item = &TextBlock> {
    blocks.iter().filter_map(|block| match block {
        Block::Text(text) => Some(text),
        _ => None,
    })
}

/// What one socket knows of the replies it streams to its page: the text of
/// each node's latest text blocks, so a tag that arrives in pieces is named
/// by the frame that completes it, and the attachments it described already,
/// so a later frame brings only what is new; an attachment belongs to the
/// conversation, so the page holds each once, whichever node names it.
#[derive(Default)]
pub struct StreamedText {
    texts: HashMap<NodeId, BTreeMap<u32, String>>,
    described: HashSet<AttachmentNumber>,
}

impl StreamedText {
    /// A reset of `node`'s transcript from `start`: the blocks it holds are
    /// the text so far, of which only the latest may still be written.
    pub fn reset(&mut self, node: &NodeId, start: u32, blocks: &[Block]) {
        let texts = self.texts.entry(node.clone()).or_default();
        texts.clear();
        if let (Some(Block::Text(last)), Ok(count)) = (blocks.last(), u32::try_from(blocks.len())) {
            texts.insert(start + count - 1, last.text.clone());
        }
    }

    /// Takes `described` as sent to the page, whichever frame carries it.
    pub fn sent(&mut self, described: &[ConversationAttachment]) {
        for attachment in described {
            if let Ok(number) = attachment.id.parse() {
                self.described.insert(number);
            }
        }
    }

    /// The attachments the replies of `node` name after `patches` apply that
    /// the page was not sent, read from `conversations` and now taken as sent.
    ///
    /// A block's text is read as it stands after the patch, whole, so a tag
    /// completed by the last of its pieces names its attachment then. A delta
    /// is read again only when it can close a tag or a line.
    pub async fn named(
        &mut self,
        conversations: &ConversationStores,
        conversation: &ConversationId,
        node: &NodeId,
        patches: &[TranscriptPatch],
    ) -> Result<Vec<ConversationAttachment>, StorageError> {
        let mut named = self.apply(node, patches);
        named.retain(|number| !self.described.contains(number));
        let found = described(conversations, conversation, named).await?;
        self.sent(&found);
        Ok(found)
    }

    /// Applies `patches` to the text kept of `node`, and answers the
    /// attachments named by the block texts they leave that may name one not
    /// named before.
    fn apply(&mut self, node: &NodeId, patches: &[TranscriptPatch]) -> BTreeSet<AttachmentNumber> {
        let texts = self.texts.entry(node.clone()).or_default();
        let mut named = BTreeSet::new();
        for patch in patches {
            match patch {
                TranscriptPatch::Add { index, value, .. } => {
                    // The blocks from the index on move one place.
                    let moved: Vec<u32> = texts.range(*index..).map(|(each, _)| *each).collect();
                    for each in moved.into_iter().rev() {
                        if let Some(text) = texts.remove(&each) {
                            texts.insert(each + 1, text);
                        }
                    }
                    if let Block::Text(text) = value {
                        texts.insert(*index, text.text.clone());
                        named.extend(block_attachments(text));
                    }
                }
                TranscriptPatch::ReplaceBlock { index, value } => match value {
                    Block::Text(text) => {
                        texts.insert(*index, text.text.clone());
                        named.extend(block_attachments(text));
                    }
                    _ => {
                        texts.remove(index);
                    }
                },
                TranscriptPatch::AppendText { index, delta } => {
                    // A block this socket holds no text of, such as a thinking
                    // block, is read from its delta alone.
                    let so_far = texts.get_mut(index);
                    let whole = match so_far {
                        Some(so_far) => {
                            so_far.push_str(delta);
                            so_far.as_str()
                        }
                        None => delta.as_str(),
                    };
                    if delta.contains(['>', '\n']) {
                        named.extend(named_attachments(whole));
                    }
                }
                TranscriptPatch::Truncate { length } => {
                    let gone: Vec<u32> = texts.range(*length..).map(|(each, _)| *each).collect();
                    for each in gone {
                        texts.remove(&each);
                    }
                }
            }
        }
        while texts.len() > KEPT_BLOCKS {
            texts.pop_first();
        }
        named
    }
}
