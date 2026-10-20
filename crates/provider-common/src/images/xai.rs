//! How xAI's image API, a Grok subscription's and an xAI key's alike,
//! refuses a shape.

use super::MadeImage;
use crate::media::{MediaAnswer, MediaFailure};

/// The shapes xAI's image API takes, as its refusal of `aspect_ratio` lists
/// them: ``aspect_ratio: unknown variant `4:5`, expected one of `1:1`,
/// `3:4`, …, `auto` at line 1 column 304``. The whole list is read, each
/// entry quoted and the last after `, ` or ` or `; only the W:H shapes are
/// kept, as `auto` is no shape `--aspect` asks for. None for any other
/// refusal or a list not read whole, which is then told in xAI's own
/// words.
fn shapes_taken(message: &str) -> Option<Vec<String>> {
    let (_, rest) = message.split_once("aspect_ratio: unknown variant `")?;
    let (_, mut rest) = rest.split_once("`, expected one of ")?;
    let mut takes = Vec::new();
    loop {
        let (entry, after) = rest.strip_prefix('`')?.split_once('`')?;
        if entry.contains(':') {
            takes.push(entry.to_owned());
        } else if entry != "auto" {
            return None;
        }
        match after.strip_prefix(", ").or_else(|| after.strip_prefix(" or ")) {
            Some(next) => rest = next,
            // The list ends where no separator follows: at the message's
            // end or its position, `at line …`.
            None if after.is_empty() || after.starts_with(" at ") => break,
            None => return None,
        }
    }
    (!takes.is_empty()).then_some(takes)
}

/// `answer` with a refusal of the shape asked for told as the shapes xAI
/// named, so a shape xAI adds needs no change.
pub fn with_shapes(mut answer: MediaAnswer<MadeImage>) -> MediaAnswer<MadeImage> {
    if let Err(MediaFailure::Refused { message, code, trace }) = &answer.result
        && let Some(takes) = shapes_taken(message)
    {
        answer.result = Err(MediaFailure::Shape { takes, code: code.clone(), trace: trace.clone() });
    }
    answer
}
