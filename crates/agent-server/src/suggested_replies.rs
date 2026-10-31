//! Suggested replies: after the root agent's turn ends with an answer, one
//! small request asks for the replies the user is likely to send next, which
//! the page offers as pills under the answer.
//!
//! The request reads what the user would: the user's recent short messages,
//! for how they write; the two exchanges before the answer, cut short; and the
//! answer's end, where its question or its offer is. Tool calls, their output
//! and thinking stay out. The backend decides when a request starts and keeps
//! what it answered (`crates/backend-user-shard/src/conversation/suggested_replies.rs`).

use std::collections::HashSet;
use std::num::NonZeroU32;

use demi_provider_common::RequestPurpose;
use demi_shared_types::{Block, BlockId, SuggestedReply, SuggestedReplyKind, TextBlock, UserContentBlock, is_blank};

use crate::small_request::SmallRequest;

/// The most suggestions a request keeps: one line of pills under the answer
/// holds three.
pub const MOST: usize = 3;

/// The longest suggestion kept, in Unicode scalar values: a pill is one line
/// of a message the user sends as is, and a longer line is a message to write,
/// not to tap.
const LONGEST: usize = 120;

/// How many of the user's recent messages the request reads for how they
/// write, and the longest of them it takes: long ones are requests, which
/// teach the model a task and not the user's voice.
const VOICE_MESSAGES: usize = 12;
const VOICE_LONGEST: usize = 300;

/// How much of each earlier exchange the request reads: the end of an
/// answer, where it asked or offered something, and the start of the
/// user's message.
const EARLIER_ANSWER_CHARS: usize = 500;
const EARLIER_MESSAGE_CHARS: usize = 400;

/// How much of the answer the request reads, from its end.
///
/// Bounded because the request goes to a small model; the end holds what
/// the user replies to.
const ANSWER_CHARS: usize = 5_000;

/// A request's cap on its output, as a title request's
/// (`crate::title::OUTPUT_CAP`): room for the lowest thinking effort of a
/// model that cannot turn thinking off, and a few short lines.
pub const OUTPUT_CAP: NonZeroU32 = NonZeroU32::new(4_096).expect("the cap is not zero");

/// The whole system prompt of a request.
///
/// Its rules come from OpenCode's opencode-ghost plugin, which asks a small
/// model for the next prompt, and from runs on a real conversation's turns:
/// without the user's recent messages the model wrote long suggestions in the
/// agent's voice, invented details the answer did not hold, and turned the
/// user's "you" into "I".
pub const INSTRUCTION: &str = "You suggest what the USER of a coding agent will most likely send next, after the assistant's latest reply. The user picks one with a single tap and it is sent as is.

Write zero to three candidates, one per line, the most likely first. Rules:
- Each candidate is a complete, specific message: an approval, a choice among the options the reply offered, a direct answer to its question, or the obvious next instruction. Never a vague one such as \"continue\", \"tell me more\" or \"what next?\".
- Use only what the reply and the transcript say: never add a version, name, number, file or step they do not mention.
- The user speaks to the assistant: \"you\" is the assistant, \"I\" is the user.
- Candidates differ in what they ask for; two ways of saying the same thing count once.
- Sound exactly like the user: copy the language, words, punctuation and length of the user's recent messages. Most are a few words; keep a candidate that short unless it must say more.
- If the reply asks for approval or recommends one option, the first candidate takes it, in the words the user uses for that.
- Start each line with its kind and a colon: `answer:` for a reply to what the assistant's reply asked or offered (an approval, a choice among its options, an answer to its question), `topic:` for something new the user may raise next. Then plain text. No numbering, quotes, markdown or explanation.
- When no message is clearly likely, such as when the reply only reports with nothing pending, or the answer needs facts only the user has, output exactly NONE.";

/// The answer the latest turn of `blocks`, the root's transcript, ended
/// with: its last text that is not blank, when the turn ended with a
/// response; none when it failed or the user stopped it. The caller tells
/// an answer written now from an earlier one by its time: a turn that ended
/// without text names an earlier turn's, and a version brought back names
/// its own.
pub fn answer_of(blocks: &[Block]) -> Option<&TextBlock> {
    match demi_agent_transcript::ended_turn_answer(blocks)? {
        (Block::Response(_), Some(text)) => Some(text),
        _ => None,
    }
}

/// One message of the conversation as the request reads it.
enum Said<'a> {
    /// A message or a steer the user wrote.
    User(String),
    /// The last text of the agent before the user wrote again.
    Agent(&'a str),
}

/// The request for the replies to the answer `answer`, a text block of
/// `blocks`, the root's transcript; none when `blocks` do not hold it or it
/// has no text.
pub fn suggestion_request(blocks: &[Block], answer: &BlockId) -> Option<SmallRequest> {
    let at = blocks.iter().position(|block| block.id() == answer)?;
    let Block::Text(latest) = &blocks[at] else {
        return None;
    };
    if is_blank(&latest.text) {
        return None;
    }
    let said = conversation(&blocks[..at]);
    let voice: Vec<String> = said
        .iter()
        .filter_map(|said| match said {
            Said::User(text) if text.chars().count() <= VOICE_LONGEST => Some(format!("- {}", one_line(text))),
            _ => None,
        })
        .rev()
        .take(VOICE_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let earlier: Vec<String> = said
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|said| match said {
            Said::User(text) => format!("User: {}", head(text, EARLIER_MESSAGE_CHARS)),
            Said::Agent(text) => format!("Assistant: {}", tail(text, EARLIER_ANSWER_CHARS)),
        })
        .collect();
    let mut message = String::new();
    if !voice.is_empty() {
        message.push_str("The user's recent messages, for how they write:\n");
        message.push_str(&voice.join("\n"));
        message.push_str("\n\n");
    }
    message.push_str("Transcript:\n\n");
    for line in &earlier {
        message.push_str(line);
        message.push_str("\n\n");
    }
    message.push_str("Assistant (latest reply): ");
    message.push_str(tail(&latest.text, ANSWER_CHARS));
    message.push_str("\n\nWrite the candidates for the user's next message.");
    Some(SmallRequest {
        instruction: INSTRUCTION,
        message,
        output_cap: OUTPUT_CAP,
        purpose: RequestPurpose::SuggestedReplies,
    })
}

/// The user's messages and the agent's answers to them, in order: each run
/// of the agent's texts is its last, the one the user replied to. A message
/// a schedule sent is not the user's.
fn conversation(blocks: &[Block]) -> Vec<Said<'_>> {
    let mut said = Vec::new();
    let mut answer: Option<&str> = None;
    for block in blocks {
        let user = match block {
            Block::User(user) if user.schedule.is_none() => Some(text_of(&user.content)),
            Block::Steer(steer) => Some(text_of(&steer.content)),
            Block::Text(text) if !is_blank(&text.text) => {
                answer = Some(&text.text);
                None
            }
            _ => None,
        };
        if let Some(text) = user.filter(|text| !is_blank(text)) {
            if let Some(answer) = answer.take() {
                said.push(Said::Agent(answer));
            }
            said.push(Said::User(text));
        }
    }
    said
}

/// The text of a message's content, its parts on lines of their own.
fn text_of(content: &[UserContentBlock]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            UserContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text`'s first `chars` characters.
fn head(text: &str, chars: usize) -> &str {
    text.char_indices().nth(chars).map_or(text, |(at, _)| &text[..at])
}

/// `text`'s last `chars` characters.
fn tail(text: &str, chars: usize) -> &str {
    let count = text.chars().count();
    if count <= chars {
        return text;
    }
    text.char_indices().nth(count - chars).map_or(text, |(at, _)| &text[at..])
}

/// The suggestions in a response: each line that is one, in order, at most
/// [`MOST`]. A line's kind is its `answer:` or `topic:` mark, an answer
/// when it has none; numbering, a list marker and quotes around it go. An
/// empty line, `NONE`, a line longer than [`LONGEST`] and a repeat are not
/// suggestions.
pub fn suggestions_from_response(response: &str) -> Vec<SuggestedReply> {
    let mut seen = HashSet::new();
    response
        .lines()
        .filter_map(suggestion)
        .filter(|reply| seen.insert(reply.text.to_lowercase()))
        .take(MOST)
        .collect()
}

fn suggestion(line: &str) -> Option<SuggestedReply> {
    let line = line.trim();
    let line = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .unwrap_or(line);
    let line = strip_number(line).trim();
    let (kind, rest) = kind_of(line);
    let text = rest
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '“' | '”' | '「' | '」'))
        .trim();
    if text.is_empty() || text.eq_ignore_ascii_case("none") || text.chars().count() > LONGEST {
        return None;
    }
    Some(SuggestedReply {
        text: text.to_owned(),
        kind,
    })
}

/// `line` without a leading `1.` or `1)`.
fn strip_number(line: &str) -> &str {
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return line;
    }
    line[digits..]
        .strip_prefix(['.', ')'])
        .unwrap_or(line)
}

/// The kind a line's mark names, and the line after it.
fn kind_of(line: &str) -> (SuggestedReplyKind, &str) {
    for (mark, kind) in [("answer", SuggestedReplyKind::Answer), ("topic", SuggestedReplyKind::Topic)] {
        if line.len() > mark.len() && line.is_char_boundary(mark.len()) && line[..mark.len()].eq_ignore_ascii_case(mark) {
            let rest = &line[mark.len()..];
            if let Some(after) = rest.strip_prefix(':').or_else(|| rest.strip_prefix('：')) {
                return (kind, after);
            }
        }
    }
    (SuggestedReplyKind::Answer, line)
}
