//! A comment on a reply's words or on a picture in it (`comments-and-ask.md`):
//! what the user selected in a text block of the agent's reply and what they
//! wrote about it, waiting in the conversation's draft and then sent with a
//! message.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{BlockId, MAX_SAFE_INTEGER, MediaSource, UserContentBlock, is_blank};

/// One comment. `block_id`, `start` and `end` say where its marks stand: the
/// reply's text block and the selection's character offsets in the block's
/// text as the page renders it. `quote` is the text it quotes, which the
/// model reads and the message's bubble shows whatever became of the block.
///
/// A comment on a picture (`comments-and-ask.md` § Images) has `picture`,
/// the picture it quotes: `start` is the picture's place among the block's
/// pictures as the page renders them, from 0, `end` the next place, and
/// `quote` the picture's alt text or name, which a reader without the
/// picture reads.
///
/// `at` is where the comment stands in the block's reading order, text and
/// pictures together: a comment on words its `start`, a comment on a
/// picture the offset in the block's rendered text where the picture stands.
/// The comments of one block go in the order of `at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageComment {
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
    #[garde(custom(not_blank))]
    pub text: String,
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

fn not_blank(text: &str, _: &()) -> garde::Result {
    if is_blank(text) {
        Err(garde::Error::new("holds no words"))
    } else {
        Ok(())
    }
}

/// The comments as the model reads them, before the message's own content:
/// each quoted text as a Markdown quote with its comment after it, a blank
/// line between them, and each quoted picture as an image the message
/// carries, then its comment; none for a message without comments.
pub fn comments_content(comments: &[MessageComment]) -> Vec<UserContentBlock> {
    let mut content = Vec::new();
    let mut text: Vec<String> = Vec::new();
    for comment in comments {
        match &comment.picture {
            None => text.push(format!("{}\n\n{}", markdown_quote(&comment.quote), comment.text)),
            Some(source) => {
                if !text.is_empty() {
                    content.push(UserContentBlock::Text { text: text.join("\n\n") });
                    text.clear();
                }
                content.push(UserContentBlock::Image { source: source.clone() });
                text.push(comment.text.clone());
            }
        }
    }
    if !text.is_empty() {
        content.push(UserContentBlock::Text { text: text.join("\n\n") });
    }
    content
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
