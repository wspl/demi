//! A comment on a reply's words or on a picture in it: what the user selected
//! in a text block of the agent's reply and what they wrote about it, waiting
//! in the conversation's draft and then sent with a message.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{BlockId, MAX_SAFE_INTEGER, MediaSource, UserContentBlock, is_blank};

/// One comment, whose content is a message's, as each stage of a message
/// holds its content: the transcript's and the queue's [`MessageComment`],
/// the frame's that sends it, and the draft's that waits.
///
/// `block_id`, `start` and `end` say where its marks stand: the reply's text
/// block and the selection's character offsets in the block's text as the
/// page renders it. `quote` is the text it quotes, which the model reads and
/// the message's bubble shows whatever became of the block.
///
/// A comment on a picture has `picture`, the picture it quotes: `start` is the
/// picture's place among the block's pictures as the page renders them, from 0,
/// `end` the next place, and `quote` the picture's alt text or name, which a
/// reader without the picture reads.
///
/// `at` is where the comment stands in the block's reading order, text and
/// pictures together: a comment on words its `start`, a comment on a
/// picture the offset in the block's rendered text where the picture stands.
/// The comments of one block go in the order of `at`.
///
/// `content` is what was written about it, text and files as a message's:
/// something, a word or a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    bound(serialize = "Stage::Content: Serialize", deserialize = "Stage::Content: Deserialize<'de>")
)]
#[schemars(rename = "{Stage}Comment", bound = "Stage: JsonSchema, Stage::Content: JsonSchema")]
#[garde(context(()))]
pub struct Comment<Stage: CommentStage> {
    /// The page's id for the comment, unique in its draft.
    #[garde(length(min = 1, max = 64))]
    pub id: String,
    #[garde(skip)]
    pub block_id: BlockId,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub start: u64,
    #[garde(range(max = MAX_SAFE_INTEGER), custom(after_start(&self.start)))]
    pub end: u64,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub at: u64,
    #[garde(length(min = 1))]
    pub quote: String,
    /// The picture a comment on a picture quotes: the blob of the user's the
    /// reply holds it as, or the URL its Markdown drew it from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub picture: Option<MediaSource>,
    #[garde(dive, custom(says_something::<Stage>))]
    pub content: Stage::Content,
}

impl<Stage: CommentStage> Comment<Stage> {
    /// The comment with `content` in place of its own, as the stage `Next` of
    /// a message holds it.
    pub fn with_content<Next: CommentStage>(self, content: Next::Content) -> Comment<Next> {
        Comment {
            id: self.id,
            block_id: self.block_id,
            start: self.start,
            end: self.end,
            at: self.at,
            quote: self.quote,
            picture: self.picture,
            content,
        }
    }
}

/// A stage of a message, which names its comments and gives the form of
/// their content, the message's own at that stage: a marker, never a value.
pub trait CommentStage: Clone + PartialEq + Eq + std::fmt::Debug {
    type Content: CommentContent;
}

/// A message's content at one stage, which its comments' content has too.
pub trait CommentContent: garde::Validate<Context = ()> + Clone + PartialEq + Eq + std::fmt::Debug {
    /// Whether it says something: text that is not blank, or any file.
    fn says_something(&self) -> bool;
}

/// A part of a message's content held as a list: its text, or a file.
pub trait CommentPart: garde::Validate<Context = ()> + Clone + PartialEq + Eq + std::fmt::Debug {
    /// Whether the part says something: text that is not blank, or any file.
    fn says_something(&self) -> bool;
}

impl<Part: CommentPart> CommentContent for Vec<Part> {
    fn says_something(&self) -> bool {
        self.iter().any(CommentPart::says_something)
    }
}

/// The stage of a message the transcript, the queue and an edit's record
/// keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message;

impl CommentStage for Message {
    type Content = Vec<UserContentBlock>;
}

impl JsonSchema for Message {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Message".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!(false)
    }
}

/// A comment as the transcript, the queue and an edit's record keep it.
pub type MessageComment = Comment<Message>;

impl CommentPart for UserContentBlock {
    fn says_something(&self) -> bool {
        match self {
            Self::Text { text } => !is_blank(text),
            _ => true,
        }
    }
}

fn after_start(start: &u64) -> impl FnOnce(&u64, &()) -> garde::Result + '_ {
    move |end, _| {
        if end > start {
            Ok(())
        } else {
            Err(garde::Error::new("ends after it starts"))
        }
    }
}

fn says_something<Stage: CommentStage>(content: &Stage::Content, _: &()) -> garde::Result {
    if content.says_something() {
        Ok(())
    } else {
        Err(garde::Error::new("holds no words and no file"))
    }
}

/// The comments as the model reads them, before the message's own content:
/// each quoted text as a Markdown quote, or each quoted picture as an image
/// the message carries, with the comment's content after it, its text joined
/// to the text around it by a blank line; none for a message without
/// comments.
pub fn comments_content(comments: &[MessageComment]) -> Vec<UserContentBlock> {
    let mut content = Vec::new();
    for comment in comments {
        match &comment.picture {
            None => push_text(&mut content, &markdown_quote(&comment.quote)),
            Some(source) => content.push(UserContentBlock::Image { source: source.clone() }),
        }
        for part in &comment.content {
            match part {
                UserContentBlock::Text { text } => push_text(&mut content, text),
                other => content.push(other.clone()),
            }
        }
    }
    content
}

/// Puts `text` after the content's last text, a blank line between them, or
/// as a text of its own after a file.
fn push_text(content: &mut Vec<UserContentBlock>, text: &str) {
    match content.last_mut() {
        Some(UserContentBlock::Text { text: last }) => {
            last.push_str("\n\n");
            last.push_str(text);
        }
        _ => content.push(UserContentBlock::Text { text: text.to_owned() }),
    }
}

/// `text` as a Markdown quote, each of its lines after `> `, an empty one as
/// `>`: how the model reads the words a comment or a side question quotes.
pub fn markdown_quote(text: &str) -> String {
    let lines: Vec<String> = text
        .lines()
        .map(|line| if line.is_empty() { ">".to_owned() } else { format!("> {line}") })
        .collect();
    lines.join("\n")
}
