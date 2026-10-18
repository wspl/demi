//! What the conversation's agent learns of its side questions
//! (`comments-and-ask.md` § What the agent learns): with the user's next
//! message or steer, each question asked aside since it last heard of
//! them, with its final answer, under the words it asked about.

use demi_agent_store::SideOf;
use demi_agent_transcript::bound_text;
use demi_shared_types::{Block, NodeId, UserContentBlock};

/// The sentence the block of the side questions source starts with.
const TOLD: &str = "The user asked these aside and was answered there; they show what the user was unsure of, for you to know, with no need to answer them again.";

/// One side question as the agent hears of it: its quoted words and each
/// question asked in it whose turn ended, the first and each follow-up, in
/// order, with the turn's final answer.
#[derive(Clone)]
pub(super) struct Asked {
    pub(super) of: SideOf,
    pub(super) pairs: Vec<(String, String)>,
}

impl Asked {
    /// The side question `id` asking `of`, from its transcript `blocks`,
    /// whose last turn `ended` or still works: the question the user typed,
    /// then each later message of its own, each with the last text its turn
    /// wrote. A turn still working, or one that wrote no text, as a failed
    /// one, is told once it answers.
    pub(super) fn of(id: &NodeId, of: SideOf, blocks: &[Block], ended: bool) -> Self {
        let Some(first) = blocks
            .iter()
            .position(|block| matches!(block, Block::User(user) if user.turn_id.as_str() == id.as_str()))
        else {
            return Self { of, pairs: Vec::new() };
        };
        let mut turns: Vec<(String, Option<&str>)> = Vec::new();
        for block in &blocks[first..] {
            match block {
                Block::User(_) if turns.is_empty() => turns.push((of.question.clone(), None)),
                Block::User(user) => turns.push((text_of(&user.content), None)),
                Block::Text(text) => {
                    if let Some(turn) = turns.last_mut() {
                        turn.1 = Some(&text.text);
                    }
                }
                _ => {}
            }
        }
        if !ended {
            turns.pop();
        }
        let pairs = turns
            .into_iter()
            .filter_map(|(question, answer)| Some((question, bound_text(answer?).into_owned())))
            .collect();
        Self { of, pairs }
    }
}

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

/// One side question's part of the block: its quoted words on one line,
/// then each of `pairs` as its question and answer.
fn section(quote: &str, pairs: &[(String, String)]) -> String {
    let quote = quote.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut text = format!("On \"{quote}\":");
    for (question, answer) in pairs {
        text.push_str(&format!("\nQ: {question}\nA: {answer}"));
    }
    text
}

/// How many of `asked`'s pairs the source's blocks `seen` told, oldest
/// first: each block tells some more of them, in a section of their own.
fn told(asked: &Asked, seen: &[&str]) -> usize {
    let mut count = 0;
    for block in seen {
        let Some(sections) = block.strip_prefix(TOLD) else {
            continue;
        };
        let rest = asked.pairs.len() - count;
        let more = (1..=rest).rev().find(|&more| {
            let told = format!("\n\n{}", section(&asked.of.quote, &asked.pairs[count..count + more]));
            sections.ends_with(&told) || sections.contains(&format!("{told}\n\n"))
        });
        count += more.unwrap_or(0);
    }
    count
}

/// The block that tells the agent the pairs of `asked`, in the order asked,
/// that its blocks `seen` did not: none when every one was told.
pub(super) fn untold(asked: &[Asked], seen: &[&str]) -> Option<String> {
    let sections: Vec<String> = asked
        .iter()
        .filter_map(|asked| {
            let count = told(asked, seen);
            (count < asked.pairs.len()).then(|| section(&asked.of.quote, &asked.pairs[count..]))
        })
        .collect();
    (!sections.is_empty()).then(|| format!("{TOLD}\n\n{}", sections.join("\n\n")))
}

/// Whether the request about to be made follows the user's new input, a
/// message or a steer: the first request of an input turn, or the first
/// after a steer, and never one in the middle of a turn.
pub(super) fn follows_input(blocks: &[Block]) -> bool {
    blocks
        .iter()
        .rev()
        .find(|block| !matches!(block, Block::Context(_) | Block::AccountSwitch(_)))
        .is_some_and(|block| matches!(block, Block::User(_) | Block::Steer(_)))
}
