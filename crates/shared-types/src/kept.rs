//! The user's own words a compaction keeps beside its summary.
//!
//! A pass keeps each of the user's messages as text that names the message it
//! came from, and chooses the newest that fit the kept words' budget. The
//! session's pass and the conversation database's migration of boundaries
//! stored before the text was kept this way choose the same.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{MAX_SAFE_INTEGER, UserContentBlock};

/// The user's words a pass keeps: at most this many estimated tokens, or this
/// share of the window in use when that is less.
const KEPT_WORDS: u64 = 20_000;
const KEPT_WORDS_PERCENT: u64 = 10;

/// A text's estimate: its UTF-8 bytes divided by 4, rounded up, so `hello`
/// and `你好` are both 2 tokens.
pub fn text_tokens(text: &str) -> u64 {
    u64::try_from(text.len()).expect("a length fits in 64 bits").div_ceil(4)
}

/// The estimated tokens of the user's words a pass keeps with a window in use
/// of `window` tokens, or none for a model that reports none.
pub fn kept_words_budget(window: Option<u64>) -> u64 {
    window.map_or(KEPT_WORDS, |window| KEPT_WORDS.min(window * KEPT_WORDS_PERCENT / 100))
}

/// A message of the user's that a compaction kept beside its summary, as
/// text that names the message it came from.
///
/// It keeps the words alone: what the message's images and attachments hold
/// stays with the message, which its index finds in the history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeptMessage {
    /// The index of the `user` or `steer` block it came from in its agent's
    /// transcript, the one `demi conversation history read` and `open` take;
    /// a message carried into a later compaction keeps it.
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub index: u64,
    /// The message's text ([`KeptMessage::of`]); the one that crossed a pass's
    /// budget is cut to it.
    #[garde(skip)]
    pub text: String,
}

impl KeptMessage {
    /// The message at `index` whose content, its preamble and comments
    /// included, is `content`.
    ///
    /// Its text and references are kept as they are, and each file in its
    /// place: an image, or an attachment of one, as `[Image]`, any other
    /// attachment as `[Attachment]`. A medium directly before an attachment is
    /// the attachment's own view, which the model read in its place, so the
    /// two are one file, as `demi conversation history open` numbers them.
    /// Text parts are joined by a line break, a placeholder by a space.
    pub fn of(index: u64, content: &[UserContentBlock]) -> Self {
        const IMAGE: &str = "[Image]";
        const ATTACHMENT: &str = "[Attachment]";
        let mut text = String::new();
        let mut after_placeholder = false;
        for (position, part) in content.iter().enumerate() {
            let viewed = matches!(content.get(position + 1), Some(UserContentBlock::Attachment(_)));
            let after_image = matches!(
                position.checked_sub(1).and_then(|before| content.get(before)),
                Some(UserContentBlock::Image { .. })
            );
            let (piece, placeholder) = match part {
                UserContentBlock::Text { text } => (text.as_str(), false),
                UserContentBlock::Reference { reference } => (reference.as_str(), false),
                UserContentBlock::Image { .. } | UserContentBlock::Video { .. } | UserContentBlock::Document { .. }
                    if viewed =>
                {
                    continue;
                }
                UserContentBlock::Image { .. } => (IMAGE, true),
                UserContentBlock::Attachment(_) if after_image => (IMAGE, true),
                UserContentBlock::Attachment(_) | UserContentBlock::Video { .. } | UserContentBlock::Document { .. } => {
                    (ATTACHMENT, true)
                }
            };
            if !text.is_empty() {
                text.push(if placeholder || after_placeholder { ' ' } else { '\n' });
            }
            text.push_str(piece);
            after_placeholder = placeholder;
        }
        Self { index, text }
    }

    /// The message as the model reads it, its own user message: its index,
    /// then its text.
    pub fn model_text(&self) -> String {
        format!("{}{}", head(self.index), self.text)
    }

    /// The message with its text cut so that what the model reads of it is at
    /// most `tokens` estimated tokens, never inside a character; none when no
    /// text is left.
    fn within(self, tokens: u64) -> Option<Self> {
        let bytes = usize::try_from(tokens.saturating_mul(4)).unwrap_or(usize::MAX);
        let room = bytes.checked_sub(head(self.index).len())?;
        let end = self.text.floor_char_boundary(room.min(self.text.len()));
        (end > 0).then(|| Self {
            index: self.index,
            text: self.text[..end].to_owned(),
        })
    }
}

/// What a kept message's text follows, which names its index.
fn head(index: u64) -> String {
    format!("[#{index}]\n")
}

/// The messages of `messages`, oldest first, that a pass keeps within
/// `budget` estimated tokens of what the model reads of them
/// ([`KeptMessage::model_text`]).
///
/// The newest are kept first, whole, while they fit. The one that crosses the
/// budget keeps its text up to it, and older ones are left to the summary.
pub fn choose_kept(messages: Vec<KeptMessage>, budget: u64) -> Vec<KeptMessage> {
    let mut kept = Vec::new();
    let mut left = budget;
    for message in messages.into_iter().rev() {
        let tokens = text_tokens(&message.model_text());
        if tokens <= left {
            left -= tokens;
            kept.push(message);
            continue;
        }
        if let Some(cut) = message.within(left) {
            kept.push(cut);
        }
        break;
    }
    kept.reverse();
    kept
}
