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
use demi_shared_types::{Block, BlockId, TextBlock, UserContentBlock, is_blank};

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
/// model for the next prompt, and from replays of real turn ends. It decides
/// first whether anything is put to the user, since a small model passes over
/// that rule among others. It never asks for a number of candidates, since a
/// model asked for several invents the rest. Each candidate names its stance so
/// that [`suggestions_from_response`] can hold what the model may not: one
/// approval and one refusal.
pub const INSTRUCTION: &str = "You suggest what the USER of a coding agent will most likely send next, after the assistant's latest reply. The user picks one with a single tap and it is sent as is.

First decide whether the latest reply is waiting for a decision from the user: it puts a question, a request for approval, a choice or an offer to the user, and the assistant will not go on until it is answered.

Output exactly NONE, and nothing else, when the reply:
- only reports, explains, advises or confirms;
- says what the assistant will do next by itself, or that it waits for work under way;
- only mentions something it could do if the user wants it, with no question (\"if you want this, tell me\");
- asks the user to do something themselves, or asks for facts only the user has: what they saw, did, have or want to describe.

Otherwise write only what the user is truly likely to send, the most likely first, one per line, each starting with its stance and a colon. One candidate is normal. Never write another just to offer more.
- `yes:` approves what the reply proposes, in the words the user uses for that.
- `no:` declines it or holds it off. Write it only when the reply itself gives a real reason to, such as a cost, a risk or a condition to wait for; never as the plain opposite of `yes:`.
- `pick:` takes one of the named options the reply offers, one line for each. With named options use `pick:` and no `yes:`, the option the reply recommends first.

Rules:
- Each candidate must make the assistant do something different from the others. Two ways of saying the same thing, or one that adds a detail to another, count once: keep the shorter.
- When the reply puts several separate decisions to the user, each candidate settles all of them. Never write one that answers one decision and leaves the others open. If no short message can settle them all, output NONE.
- A candidate is a decision, never a statement of fact about the user: not what they did, saw, have or know.
- Use only what the reply says: never add a version, name, number, file, step or new work it does not mention.
- The user speaks to the assistant: \"you\" is the assistant, \"I\" is the user.
- Sound exactly like the user: copy the language, words, punctuation and length of the user's recent messages. Most are a few words; keep a candidate that short.
- No numbering, quotes, markdown or explanation.";

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
/// [`MOST`]. A line starts with its stance, of which `yes:` and `no:` count once;
/// numbering, a list marker and quotes around it go. A line without a stance, and
/// so `NONE`, an empty one, one longer than [`LONGEST`] and a repeat are not
/// suggestions.
pub fn suggestions_from_response(response: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut stances = HashSet::new();
    response
        .lines()
        .filter_map(suggestion)
        .filter(|(_, text)| seen.insert(text.to_lowercase()))
        .filter(|(stance, _)| *stance == Stance::Pick || stances.insert(*stance))
        .map(|(_, text)| text)
        .take(MOST)
        .collect()
}

/// What a candidate does with what the answer put to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Stance {
    /// Approves what it proposes.
    Yes,
    /// Declines it or holds it off.
    No,
    /// Takes one of the options it names.
    Pick,
}

fn suggestion(line: &str) -> Option<(Stance, String)> {
    let line = line.trim();
    let line = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .unwrap_or(line);
    let line = strip_number(line).trim();
    let (stance, rest) = stance_of(line)?;
    let text = rest
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '“' | '”' | '「' | '」'))
        .trim();
    if text.is_empty() || text.chars().count() > LONGEST {
        return None;
    }
    Some((stance, text.to_owned()))
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

/// The stance a line starts with, and the line after it.
fn stance_of(line: &str) -> Option<(Stance, &str)> {
    [("yes", Stance::Yes), ("no", Stance::No), ("pick", Stance::Pick)]
        .into_iter()
        .find_map(|(mark, stance)| {
            let head = line.get(..mark.len()).filter(|head| head.eq_ignore_ascii_case(mark))?;
            let rest = &line[head.len()..];
            let after = rest.strip_prefix(':').or_else(|| rest.strip_prefix('：'))?;
            Some((stance, after))
        })
}
