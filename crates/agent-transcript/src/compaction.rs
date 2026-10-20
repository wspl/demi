//! What a compaction pass reads of the blocks.
//!
//! It reads two things: the window it summarizes, which ends where the content
//! of a request the vendor answered ends, and the user's own messages it keeps
//! beside the summary, each as text that names the message it came from. Each
//! is a pure reading of the blocks: the window weighed by the token estimates
//! of the request the pass serves, the kept messages by the text the model
//! reads of them, as `demi_shared_types::choose_kept` chooses them.

use demi_shared_types::{Block, KeptMessage, ModelSelection, choose_kept, comments_content};

use super::{
    estimate::block_tokens,
    is_answer, is_compaction,
    replay::RequestView,
    replay_start,
};

/// What the model reads first of a `compaction_boundary`, before the
/// messages it kept and its summary.
pub const COMPACTION_NOTE: &str = "The earlier part of this conversation was compacted. The user's messages from it follow as they were written, then a summary of the rest. Each kept message starts with its index, which `demi conversation history read` and `open` take. Continue the work from where it stands; do not mention or restate the summary. What the summary leaves out is still in the conversation: `demi conversation history` finds and reads it.";

/// The summary of a `compaction_boundary` as the model reads it: in
/// `<summary>` tags.
pub fn summary_text(summary: &str) -> String {
    format!("<summary>\n{summary}\n</summary>")
}

/// The blocks a pass summarizes: from the last `compaction_boundary`, or the
/// first block, to the cut point, where the content of an answered request
/// ends.
///
/// With no request answered since the last compaction, the blocks end where the
/// input no request answered begins, which the pass keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionWindow {
    pub start: usize,
    pub cut: usize,
}

/// The window of the next pass over the blocks of `request`, a request of
/// `model`, which keeps at least `budget` estimated tokens after its cut where
/// it can.
///
/// The cut is:
///
/// 1. the latest end of a request `model` answered that leaves the budget after
///    it;
/// 2. otherwise the latest end of a request `model` answered;
/// 3. when `model` answered none since the last compaction, as right after a
///    switch, the latest answered request's end, whichever model answered it.
///
/// An end of a request's content never parts a call from its result: a
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

/// Where the content of each request answered since the last compaction ends,
/// in order.
///
/// Each `response` block after the last boundary and its marker closes an
/// answer, the run of thinking, text and tool-call blocks directly before it.
/// What the blocks before that answer replay is what its request carried. A
/// boundary whose marker an edit cut counts as the last compaction.
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

/// Where the input no request has answered begins.
///
/// It is the last `user` block after the latest `response` block, or right
/// after that response when no `user` block follows it. Without a response, it
/// is the last `user` block, or the first block.
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

/// The user's own messages a pass over `window`, whose first block is at
/// `first` in the transcript, keeps beside its summary within `budget`,
/// oldest first ([`choose_kept`]).
///
/// They are chosen among, in order:
///
/// 1. the messages the boundary that opens the window kept, with their
///    indices;
/// 2. the window's `user` and `steer` blocks, each named by its index
///    ([`KeptMessage::of`]).
///
/// A receipt of the user's decision, an agent's message and a command report
/// are not the user's words.
pub fn kept_messages(window: &[Block], first: usize, budget: u64) -> Vec<KeptMessage> {
    let mut messages: Vec<KeptMessage> = Vec::new();
    for (offset, block) in window.iter().enumerate() {
        let index = u64::try_from(first + offset).expect("a transcript index fits in 64 bits");
        match block {
            Block::CompactionBoundary(boundary) => messages.extend(boundary.kept.iter().cloned()),
            Block::User(user) => {
                let content: Vec<_> = user.lead_content().into_iter().chain(user.content.iter().cloned()).collect();
                messages.push(KeptMessage::of(index, &content));
            }
            Block::Steer(steer) => {
                let content: Vec<_> =
                    comments_content(&steer.comments).into_iter().chain(steer.content.iter().cloned()).collect();
                messages.push(KeptMessage::of(index, &content));
            }
            _ => {}
        }
    }
    choose_kept(messages, budget)
}
