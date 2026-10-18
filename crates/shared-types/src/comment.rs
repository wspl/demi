//! A comment on a reply's words (`comments-and-ask.md`): the text a user
//! selected in a text block of the agent's reply and what they wrote about
//! it, waiting in the conversation's draft and then sent with a message.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{BlockId, MAX_SAFE_INTEGER, is_blank};

/// One comment. `block_id`, `start` and `end` say where its marks stand: the
/// reply's text block and the selection's character offsets in the block's
/// text as the page renders it. `quote` is the text it quotes, which the
/// model reads and the message's bubble shows whatever became of the block.
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
    #[garde(length(min = 1))]
    pub quote: String,
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

/// The comments as the model reads them, before the message's own text:
/// each quoted text as a Markdown quote, its comment after it, a blank line
/// between them; none for a message without comments.
pub fn comments_markdown(comments: &[MessageComment]) -> Option<String> {
    if comments.is_empty() {
        return None;
    }
    let pairs: Vec<String> = comments
        .iter()
        .map(|comment| {
            let quote: Vec<String> = comment
                .quote
                .lines()
                .map(|line| if line.is_empty() { ">".to_owned() } else { format!("> {line}") })
                .collect();
            format!("{}\n\n{}", quote.join("\n"), comment.text)
        })
        .collect();
    Some(pairs.join("\n\n"))
}
