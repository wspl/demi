//! What a compaction pass reads of the blocks (`compaction.md` § One pass):
//! the window it summarizes, which ends where the content of a request the
//! vendor answered ends, and the user's own messages it keeps beside the
//! summary. Each is a pure reading of the blocks, weighed by the token
//! estimates of the request the pass serves.

use demi_shared_types::{Block, KeptMessage, ModelSelection, UserContentBlock};

use super::{
    estimate::{block_tokens, content_tokens},
    is_answer, is_compaction,
    replay::RequestView,
    replay_start,
};

/// What the model reads first of a `compaction_boundary`, before the
/// messages it kept and its summary (`compaction.md` § What the model
/// receives afterward).
pub const COMPACTION_NOTE: &str = "The earlier part of this conversation was compacted. The user's messages from it follow as they were written, then a summary of the rest. Continue the work from where it stands; do not mention or restate the summary.";

/// The summary of a `compaction_boundary` as the model reads it: in
/// `<summary>` tags.
pub fn summary_text(summary: &str) -> String {
    format!("<summary>\n{summary}\n</summary>")
}

/// The blocks a pass summarizes: from the last `compaction_boundary`, or
/// the first block, to the cut point, where the content of an answered
/// request ends; with no request answered since the last compaction, to
/// where the input no request answered begins, which the pass keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionWindow {
    pub start: usize,
    pub cut: usize,
}

/// The window of the next pass over the blocks of `request`, a request of
/// `model`, which keeps at least `budget` estimated tokens after its cut
/// where it can. The cut is the latest end of a request `model` answered
/// that leaves the budget after it, or the latest such end when none does;
/// when `model` answered none since the last compaction, as right after a
/// switch, the latest answered request's end, whichever model answered
/// it. An end of a request's content never parts a call from its result: a
/// `tool_call` block holds both.
pub fn compaction_window(
    request: &RequestView,
    model: &ModelSelection,
    budget: u64,
) -> CompactionWindow {
    let blocks = request.blocks();
    let start = replay_start(blocks);
    let answered = answered_requests(blocks);
    let Some(latest) = answered.last().map(|(end, _)| *end) else {
        return CompactionWindow {
            start,
            cut: unanswered_input(blocks).max(start),
        };
    };
    let ends: Vec<usize> = answered
        .iter()
        .filter(|(_, by)| same_model(by, model))
        .map(|(end, _)| *end)
        .collect();
    let Some(&current) = ends.last() else {
        return CompactionWindow { start, cut: latest };
    };
    // The estimates after each end, summed from the last block back.
    let mut after = 0;
    let mut from = blocks.len();
    for &end in ends.iter().rev() {
        after += blocks[end..from]
            .iter()
            .map(|block| block_tokens(block, request))
            .sum::<u64>();
        from = end;
        if after >= budget {
            return CompactionWindow { start, cut: end };
        }
    }
    CompactionWindow { start, cut: current }
}

/// Whether two selections name the same model of the same provider entry,
/// whose vendor caches the requests of either.
fn same_model(one: &ModelSelection, other: &ModelSelection) -> bool {
    one.provider_id == other.provider_id && one.model.id == other.model.id
}

/// Where the content of each request answered since the last compaction
/// ends, in order (`runtime.md` § Replay): each `response` block after the
/// last boundary and its marker closes an answer, the run of thinking, text
/// and tool-call blocks directly before it, and what the blocks before that
/// answer replay is what its request carried. A boundary whose marker an
/// edit cut counts as the last compaction.
pub fn answered_ends(blocks: &[Block]) -> Vec<usize> {
    answered_requests(blocks)
        .into_iter()
        .map(|(end, _)| end)
        .collect()
}

/// The same, each with the model whose `response` block closed it.
fn answered_requests(blocks: &[Block]) -> Vec<(usize, &ModelSelection)> {
    let floor = blocks
        .iter()
        .rposition(is_compaction)
        .map_or(0, |compaction| compaction + 1);
    let mut answered: Vec<(usize, &ModelSelection)> = Vec::new();
    for (index, block) in blocks.iter().enumerate().skip(floor) {
        let Block::Response(response) = block else {
            continue;
        };
        let end = blocks[floor..index]
            .iter()
            .rposition(|block| !is_answer(block))
            .map_or(floor, |before| floor + before + 1);
        if answered.last().map(|(last, _)| *last) != Some(end) {
            answered.push((end, &response.model));
        }
    }
    answered
}

/// Where the input no request has answered begins: at the last `user` block
/// after the latest `response` block, or right after that response when no
/// `user` block follows it; without a response, at the last `user` block,
/// or at the first block.
fn unanswered_input(blocks: &[Block]) -> usize {
    let answered = blocks
        .iter()
        .rposition(|block| matches!(block, Block::Response(_)))
        .map_or(0, |response| response + 1);
    blocks[answered..]
        .iter()
        .rposition(|block| matches!(block, Block::User(_)))
        .map_or(answered, |user| answered + user)
}

/// The user's own messages a pass over `window`, blocks of `request`, keeps
/// beside its summary, oldest first (`compaction.md` § One pass): of the
/// messages the boundary that opens the window kept, then the window's
/// `user` and `steer` blocks, the newest first, whole, while they fit
/// `budget` estimated tokens; the one that crosses it keeps its text up to
/// the budget, without its media, and older ones are left to the summary.
/// A receipt of the user's decision, an agent's message and a command
/// report are not the user's words.
pub fn kept_messages(window: &[Block], request: &RequestView, budget: u64) -> Vec<KeptMessage> {
    let mut messages: Vec<KeptMessage> = Vec::new();
    for block in window {
        match block {
            Block::CompactionBoundary(boundary) => messages.extend(boundary.kept.iter().cloned()),
            Block::User(user) => {
                let preamble = user.preamble.iter().map(|text| UserContentBlock::Text {
                    text: text.clone(),
                });
                messages.push(KeptMessage {
                    content: preamble.chain(user.content.iter().cloned()).collect(),
                });
            }
            Block::Steer(steer) => messages.push(KeptMessage {
                content: steer.content.clone(),
            }),
            _ => {}
        }
    }
    let mut kept = Vec::new();
    let mut left = budget;
    for message in messages.into_iter().rev() {
        let tokens = content_tokens(&message.content, request);
        if tokens <= left {
            left -= tokens;
            kept.push(message);
            continue;
        }
        if let Some(cut) = text_within(&message.content, left) {
            kept.push(cut);
        }
        break;
    }
    kept.reverse();
    kept
}

/// The text parts of `content`, cut to at most `tokens` estimated tokens
/// together, as a message's estimate counts them (one line per part), never
/// inside a character; none when no text is left.
fn text_within(content: &[UserContentBlock], tokens: u64) -> Option<KeptMessage> {
    let mut bytes = usize::try_from(tokens.saturating_mul(4)).unwrap_or(usize::MAX);
    let mut cut = Vec::new();
    for part in content {
        let UserContentBlock::Text { text } = part else {
            continue;
        };
        // The line break before every part after the first.
        if !cut.is_empty() {
            if bytes == 0 {
                break;
            }
            bytes -= 1;
        }
        let end = text.floor_char_boundary(bytes.min(text.len()));
        if end == 0 {
            break;
        }
        bytes -= end;
        cut.push(UserContentBlock::Text {
            text: text[..end].to_owned(),
        });
        if end < text.len() {
            break;
        }
    }
    (!cut.is_empty()).then_some(KeptMessage { content: cut })
}
