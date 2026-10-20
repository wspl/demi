//! What fills the context: the rows of the context card, one figure each,
//! computed from what the session holds, with no request to count.
//! Messages is the rest of the estimate after the rows above it, so the
//! rows add up to the estimate by construction.

use std::collections::HashSet;

use demi_agent_store::media::{ModelView, references};
use demi_agent_transcript::estimate::{block_tokens, prompt_tokens, text_tokens};
use demi_shared_types::{Block, ContextRow, ContextRowKind, INSTRUCTIONS_SOURCE, SKILLS_SOURCE};

use super::{
    compaction::token_threshold,
    core::SessionCore,
    runtime::{PromptLayer, SystemPrompt},
};

/// The rows of `core`'s next request with `prompt`, whose text is
/// `prompt_text`, and whose estimate is `estimate`, in `window`, the window
/// in use, in the order of `ContextRowKind`, the rows with no tokens left
/// out. A replayed block whose media the session holds nothing for, as
/// after a restore, cannot be weighed: what it weighs stays in Messages.
pub(super) fn rows(
    core: &SessionCore,
    prompt: &SystemPrompt,
    prompt_text: &str,
    estimate: u64,
    window: Option<u64>,
) -> Vec<ContextRow> {
    let view = weighable_view(core);
    let request = core.request_view(&view);
    let mut fill = Fill {
        prompt_tokens: prompt_tokens(prompt_text, &core.tools),
        commands: prompt
            .layers
            .iter()
            .filter_map(|layer| match layer {
                PromptLayer::Commands(text) => Some(text_tokens(text)),
                PromptLayer::Text(_) => None,
            })
            .sum(),
        ..Fill::default()
    };
    for block in &view.blocks {
        fill.add(block, block_tokens(block, &request));
    }
    fill.rows(estimate, window.map(|window| (window, core.model.model.output_limit)))
}

/// The view of the replayed blocks the session can weigh: all of them, or
/// without those that reference a medium it holds nothing for.
fn weighable_view(core: &SessionCore) -> ModelView {
    let start = core.transcript.replay_start();
    let replayed = core.replayed();
    match ModelView::of(start, replayed, &core.media) {
        Ok(view) => view,
        Err(unheld) => {
            let unheld: HashSet<_> = unheld.into_iter().collect();
            let weighable: Vec<Block> = replayed
                .iter()
                .filter(|block| !references(block).any(|blob| unheld.contains(blob)))
                .cloned()
                .collect();
            ModelView::of(start, &weighable, &core.media)
                .expect("the session holds every medium these blocks reference")
        }
    }
}

/// The rows' tokens found so far.
#[derive(Default)]
struct Fill {
    /// The estimate of the whole system prompt and the tools, which the
    /// System prompt and Commands rows share, as the estimate without an
    /// anchor counts them.
    prompt_tokens: u64,
    /// The capability index's estimate, with its heading.
    commands: u64,
    instructions: u64,
    skills: u64,
    other_context: u64,
    summary: u64,
}

impl Fill {
    /// Counts `block`, which weighs `tokens`. Every block no row above
    /// Messages names is in Messages, the rest of the estimate.
    fn add(&mut self, block: &Block, tokens: u64) {
        match block {
            Block::Context(context) if context.source == INSTRUCTIONS_SOURCE => {
                self.instructions += tokens;
            }
            Block::Context(context) if context.source == SKILLS_SOURCE => self.skills += tokens,
            Block::Context(_) => self.other_context += tokens,
            Block::CompactionBoundary(_) => self.summary += tokens,
            _ => {}
        }
    }

    /// The rows for an estimate of `estimate`, in a window in use with the
    /// output limit of its model, when it has one.
    fn rows(self, estimate: u64, window: Option<(u64, Option<u32>)>) -> Vec<ContextRow> {
        let carried = [
            (
                ContextRowKind::SystemPrompt,
                self.prompt_tokens.saturating_sub(self.commands),
            ),
            (ContextRowKind::Commands, self.commands),
            (ContextRowKind::Instructions, self.instructions),
            (ContextRowKind::Skills, self.skills),
            (ContextRowKind::OtherContext, self.other_context),
            (ContextRowKind::Summary, self.summary),
        ];
        let above: u64 = carried.iter().map(|(_, tokens)| tokens).sum();
        let messages = estimate.saturating_sub(above);
        let row = |kind, tokens, estimated| ContextRow {
            kind,
            tokens,
            estimated,
        };
        let mut rows = vec![row(ContextRowKind::Messages, messages, true)];
        rows.extend(carried.map(|(kind, tokens)| row(kind, tokens, true)));
        if let Some((window, output_limit)) = window {
            let buffer = window - token_threshold(window, output_limit);
            let filled = above + messages;
            rows.push(row(ContextRowKind::CompactionBuffer, buffer, false));
            rows.push(row(
                ContextRowKind::FreeSpace,
                window.saturating_sub(filled + buffer),
                false,
            ));
        }
        rows.retain(|row| row.tokens > 0);
        rows
    }
}
