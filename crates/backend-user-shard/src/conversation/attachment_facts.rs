//! The attachments and the plugins' reply blocks a transcript's replies
//! name, which travel beside the blocks as failure facts do, never stored
//! with them.
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
//! its own delta: [`StreamedText`] keeps what a socket needs of it. It knows
//! the text from the pieces it sent, so a tag split between the page's read of
//! the block and the next piece is named by neither; the page shows that
//! place as missing until it reads the block again.
//!
//! A reply block, such as a widget, is named by its tag's line as an
//! attachment is, for the tags the plugins' pages draw, and its facts are
//! what its plugin answers for it, on or off
//! (`demi_plugin_host::UserPlugins::reply_block_facts`). They never change,
//! so a socket sends each block's once.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use demi_backend_database::StorageError;
use demi_backend_database::conversation_attachments::{AttachmentNumber, read_all};
use demi_backend_database::conversations::ConversationStores;
use demi_backend_database::shares::{block_attachments, named_attachments, named_reply_blocks};
use demi_conversation_lane_protocol::{ConversationAttachment, ReplyBlockFacts, TranscriptPatch};
use demi_plugin_host::UserPlugins;
use demi_shared_types::{Block, NodeId, TextBlock};
use demi_web_api_protocol::ids::ConversationId;

/// How many text blocks of a node a socket keeps the text of: the one being
/// written and the few before it.
///
/// Bounded because the socket holds them in memory for as long as it lives.
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

/// The reply blocks `replies`, the replies' text blocks, name, by tag and
/// ID: read from their text alone, so a page that names none asks no plugin.
pub fn named_blocks<'a>(replies: impl IntoIterator<Item = &'a TextBlock>) -> BTreeSet<(String, String)> {
    replies.into_iter().flat_map(|text| named_reply_blocks(&text.text)).collect()
}

/// The facts of the reply blocks `named`, which their plugins answer, in
/// their tags' and IDs' order; a plugin that fails leaves its blocks out,
/// which their places show as missing.
pub async fn reply_block_facts(plugins: &UserPlugins, named: BTreeSet<(String, String)>) -> Vec<ReplyBlockFacts> {
    let mut by_tag: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (tag, id) in named {
        by_tag.entry(tag).or_default().push(id);
    }
    let mut out = Vec::new();
    for (tag, ids) in by_tag {
        match plugins.reply_block_facts(&tag, ids).await {
            Ok(Some(facts)) => out.extend(facts.into_iter().map(|(id, facts)| ReplyBlockFacts { tag: tag.clone(), id, facts })),
            Ok(None) => {}
            Err(error) => tracing::warn!(
                event = "reply_block_facts.read_failed",
                %tag,
                error = &error as &dyn std::error::Error,
                "the facts of the reply blocks a transcript names were not read"
            ),
        }
    }
    out
}

/// The replies among `blocks`.
pub fn replies(blocks: &[Block]) -> impl Iterator<Item = &TextBlock> {
    blocks.iter().filter_map(|block| match block {
        Block::Text(text) => Some(text),
        _ => None,
    })
}

/// What one socket knows of the replies it streams to its page: the text of
/// each node's latest text blocks it saw, so a tag that arrives in pieces is named
/// by the frame that completes it, and the attachments it described already,
/// so a later frame brings only what is new; an attachment belongs to the
/// conversation, so the page holds each once, whichever node names it.
#[derive(Default)]
pub struct StreamedText {
    texts: HashMap<NodeId, BTreeMap<u32, String>>,
    described: HashSet<AttachmentNumber>,
    /// The reply blocks whose facts it sent, by tag and ID.
    told: HashSet<(String, String)>,
}

impl StreamedText {
    /// Takes `described` as sent to the page, whichever frame carries it.
    pub fn sent(&mut self, described: &[ConversationAttachment]) {
        for attachment in described {
            if let Ok(number) = attachment.id.parse() {
                self.described.insert(number);
            }
        }
    }

    /// Takes the facts of `blocks` as sent to the page. A block with none is
    /// asked again when a reply names it again: a widget may be created after
    /// a reply named it, and its facts then come, which never change after.
    pub fn told(&mut self, blocks: &[ReplyBlockFacts]) {
        self.told.extend(
            blocks
                .iter()
                .filter(|block| !block.facts.is_null())
                .map(|block| (block.tag.clone(), block.id.clone())),
        );
    }

    /// The attachments and the reply blocks the replies of `node` name after
    /// `patches` apply that the page was not sent, read from `conversations`
    /// and from `plugins`, and now taken as sent.
    ///
    /// A block's text is read as it stands after the patch, whole, so a tag
    /// completed by the last of its pieces names its attachment or block then.
    /// A delta is read again only when it can close a tag or a line.
    pub async fn named(
        &mut self,
        conversations: &ConversationStores,
        plugins: &UserPlugins,
        conversation: &ConversationId,
        node: &NodeId,
        patches: &[TranscriptPatch],
    ) -> (Result<Vec<ConversationAttachment>, StorageError>, Vec<ReplyBlockFacts>) {
        let (mut named, mut blocks) = self.apply(node, patches);
        named.retain(|number| !self.described.contains(number));
        blocks.retain(|block| !self.told.contains(block));
        let found = described(conversations, conversation, named).await;
        if let Ok(found) = &found {
            self.sent(found);
        }
        let facts = reply_block_facts(plugins, blocks).await;
        self.told(&facts);
        (found, facts)
    }

    /// Applies `patches` to the text kept of `node`, and answers the
    /// attachments and the reply blocks named by the block texts they leave
    /// that may name one not named before.
    fn apply(
        &mut self,
        node: &NodeId,
        patches: &[TranscriptPatch],
    ) -> (BTreeSet<AttachmentNumber>, BTreeSet<(String, String)>) {
        let texts = self.texts.entry(node.clone()).or_default();
        let mut named = BTreeSet::new();
        let mut blocks = BTreeSet::new();
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
                        blocks.extend(named_reply_blocks(&text.text));
                    }
                }
                TranscriptPatch::ReplaceBlock { index, value } => match value {
                    Block::Text(text) => {
                        texts.insert(*index, text.text.clone());
                        named.extend(block_attachments(text));
                        blocks.extend(named_reply_blocks(&text.text));
                    }
                    _ => {
                        texts.remove(index);
                    }
                },
                TranscriptPatch::AppendText { index, delta, .. } => {
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
                    // A tag stands alone on its line, so the text is read
                    // again only when the delta can close one: its lines,
                    // from the start of the one it continues, hold a `<`.
                    let closes = delta.contains(['>', '\n']) && {
                        let start = whole.len() - delta.len();
                        let line = whole[..start].rfind('\n').map_or(0, |found| found + 1);
                        whole[line..].contains('<')
                    };
                    if closes {
                        named.extend(named_attachments(whole));
                        blocks.extend(named_reply_blocks(whole));
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
        (named, blocks)
    }
}
