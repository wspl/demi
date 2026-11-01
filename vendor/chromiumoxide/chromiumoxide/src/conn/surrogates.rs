//! Chrome's JSON with every lone surrogate made U+FFFD (Demi patch).
//!
//! Chrome passes a page's strings on as they are, so a string the page cut
//! between the two halves of an emoji reaches the protocol as an escape such
//! as `"\ud83d"`. That is valid JSON, which JavaScript's `JSON.parse` reads,
//! but serde_json refuses it, and a message the connection cannot read fails
//! the whole browser. Chrome's UTF-8 cannot hold a lone surrogate unescaped,
//! so only escapes are looked at.

use std::borrow::Cow;

/// `text` with each escaped UTF-16 surrogate that is not half of a pair
/// written as `�`, the replacement character.
pub(crate) fn well_formed(text: &str) -> Cow<'_, str> {
    // Most messages hold no surrogate escape at all.
    if !text.contains("\\ud") && !text.contains("\\uD") {
        return Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut written = String::new();
    let mut copied = 0;
    let mut at = 0;
    while let Some(offset) = bytes
        .get(at..)
        .and_then(|rest| rest.iter().position(|&byte| byte == b'\\'))
    {
        let escape = at + offset;
        let Some(unit) = escaped_unit(bytes, escape) else {
            // `\\`, `\"` and the other one-letter escapes.
            at = escape + 2;
            continue;
        };
        at = escape + 6;
        let lone = match unit {
            0xD800..=0xDBFF => {
                if escaped_unit(bytes, at).is_some_and(|next| (0xDC00..=0xDFFF).contains(&next)) {
                    at += 6;
                    false
                } else {
                    true
                }
            }
            0xDC00..=0xDFFF => true,
            _ => false,
        };
        if lone {
            written.push_str(&text[copied..escape]);
            written.push_str("\\ufffd");
            copied = at;
        }
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    written.push_str(&text[copied..]);
    Cow::Owned(written)
}

/// The UTF-16 unit of the `\uXXXX` escape at `at`, if one is there.
fn escaped_unit(bytes: &[u8], at: usize) -> Option<u16> {
    let escape = bytes.get(at..at + 6)?;
    if escape[..2] != *b"\\u" {
        return None;
    }
    u16::from_str_radix(std::str::from_utf8(&escape[2..]).ok()?, 16).ok()
}
