//! What fills the context (`compaction.md` § What fills the context): the
//! rows of the context card, computed from what the session holds, with no
//! request to count. Messages is the rest of the estimate after the rows
//! above it, so the rows add up to the estimate by construction.

use std::collections::{BTreeMap, HashSet};

use demi_agent_store::media::{ModelView, references};
use demi_agent_transcript::{
    estimate::{BlockWeight, block_weight, prompt_tokens, text_tokens, tool_tokens},
    replay_start, tool_input,
};
use demi_shared_types::{
    Block, ContextBlock, ContextPart, ContextRow, ContextRowKind, INSTRUCTIONS_SOURCE,
    InstructionEntry, SKILLS_SOURCE, ToolCallBlock, instruction_sections,
};

use super::{
    compaction::token_threshold,
    core::SessionCore,
    runtime::{PromptLayer, SystemPrompt},
};

/// What calls that run no `demi` command group are counted under.
const SHELL_GROUP: &str = "shell";

/// The rows of `core`'s next request with `prompt`, whose text is
/// `prompt_text`, and whose estimate is `estimate`, in `window`, the window in use, in the card's order, the
/// rows with no tokens left out. A replayed block whose media the session
/// holds nothing for, as after a restore, cannot be weighed: what it weighs
/// stays in Messages, as Unattributed.
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
        ..Fill::default()
    };
    for layer in &prompt.layers {
        match layer {
            PromptLayer::Text { kind, text } => fill.system_prompt.push(ContextPart::Prompt {
                layer: *kind,
                tokens: text_tokens(text),
            }),
            PromptLayer::Commands { text, index } => {
                fill.commands_tokens += text_tokens(text);
                for group in &index.groups {
                    fill.commands.push(ContextPart::CommandGroup {
                        group: group.path.clone(),
                        tokens: text_tokens(&group.section),
                    });
                }
            }
        }
    }
    for tool in core.tools.iter() {
        fill.system_prompt.push(ContextPart::Tool {
            name: tool.name.clone(),
            tokens: tool_tokens(tool),
        });
    }
    let groups: Vec<&str> = prompt
        .layers
        .iter()
        .flat_map(|layer| match layer {
            PromptLayer::Commands { index, .. } => index.groups.as_slice(),
            PromptLayer::Text { .. } => &[],
        })
        .map(|group| group.path.as_str())
        .collect();
    for block in &view.blocks {
        fill.add(block, block_weight(block, &request), &groups);
    }
    fill.rows(estimate, window.map(|window| (window, core.model.model.output_limit)))
}

/// The view of the replayed blocks the session can weigh: all of them, or
/// without those that reference a medium it holds nothing for.
fn weighable_view(core: &SessionCore) -> ModelView {
    let blocks = core.transcript.blocks();
    let start = replay_start(blocks);
    let replayed = &blocks[start..];
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

/// The parts found so far, by row.
#[derive(Default)]
struct Fill {
    /// The estimate of the whole system prompt and the tools, which the
    /// System prompt and Commands rows share, as the estimate without an
    /// anchor counts them.
    prompt_tokens: u64,
    system_prompt: Vec<ContextPart>,
    /// The capability index's estimate, with its heading and opening
    /// paragraphs, and its groups.
    commands_tokens: u64,
    commands: Vec<ContextPart>,
    instructions: Vec<(InstructionEntry, u64)>,
    instructions_tokens: u64,
    skills: Vec<ContextPart>,
    skills_tokens: u64,
    sources: BTreeMap<String, u64>,
    summary: u64,
    user_messages: u64,
    agent_text: u64,
    thinking: u64,
    images: u64,
    /// Command reports, agent messages and the records of a turn.
    reports: u64,
    calls: BTreeMap<String, u64>,
}

impl Fill {
    /// Counts `block`, which weighs `weight`; `groups` are the command
    /// groups of the capability index.
    fn add(&mut self, block: &Block, weight: BlockWeight, groups: &[&str]) {
        match block {
            Block::Context(context) if context.source == INSTRUCTIONS_SOURCE => {
                self.add_instructions(context, weight.tokens());
            }
            Block::Context(context) if context.source == SKILLS_SOURCE => {
                self.skills_tokens += weight.tokens();
                self.skills.extend(skill_parts(&context.text));
            }
            Block::Context(context) => {
                *self.sources.entry(context.source.clone()).or_default() += weight.tokens();
            }
            Block::CompactionBoundary(_) => self.summary += weight.tokens(),
            Block::User(_) | Block::Steer(_) => {
                self.user_messages += weight.text;
                self.images += weight.media;
            }
            Block::Text(_) => self.agent_text += weight.text,
            Block::Thinking(_) | Block::RedactedThinking(_) => self.thinking += weight.text,
            Block::ToolCall(call) => {
                *self.calls.entry(call_group(call, groups)).or_default() += weight.text;
                self.images += weight.media;
            }
            Block::Wakeup(_) => {
                self.reports += weight.text;
                self.images += weight.media;
            }
            Block::AgentMessage(_)
            | Block::Resume(_)
            | Block::Response(_)
            | Block::Error(_)
            | Block::Abort(_)
            | Block::CompactionMarker(_) => self.reports += weight.text,
        }
    }

    /// Counts an instructions block of `tokens`, each entry by its section.
    fn add_instructions(&mut self, context: &ContextBlock, tokens: u64) {
        self.instructions_tokens += tokens;
        for (entry, section) in instruction_sections(&context.text, &context.instructions) {
            let tokens = text_tokens(section);
            match self.instructions.iter_mut().find(|(seen, _)| seen == entry) {
                Some((_, sum)) => *sum += tokens,
                None => self.instructions.push((entry.clone(), tokens)),
            }
        }
    }

    /// The rows for an estimate of `estimate`, in a window in use with the
    /// output limit of its model, when it has one.
    fn rows(self, estimate: u64, window: Option<(u64, Option<u32>)>) -> Vec<ContextRow> {
        let system_prompt_tokens = self.prompt_tokens.saturating_sub(self.commands_tokens);
        let sources_tokens = self.sources.values().sum();
        let above = [
            system_prompt_tokens,
            self.commands_tokens,
            self.instructions_tokens,
            self.skills_tokens,
            sources_tokens,
            self.summary,
        ]
        .iter()
        .sum::<u64>();
        let messages = estimate.saturating_sub(above);
        let mut message_parts = vec![
            ContextPart::UserMessages {
                tokens: self.user_messages,
            },
            ContextPart::AgentText {
                tokens: self.agent_text,
            },
            ContextPart::Thinking {
                tokens: self.thinking,
            },
            ContextPart::Images {
                tokens: self.images,
            },
            ContextPart::Reports {
                tokens: self.reports,
            },
        ];
        message_parts.extend(
            self.calls
                .into_iter()
                .map(|(group, tokens)| ContextPart::CommandGroup { group, tokens }),
        );
        // Parts that add up to more than the rest, as when the provider
        // counts less than the text estimates, are scaled down to it.
        let message_parts = ContextPart::scaled_to(message_parts, messages);
        let attributed: u64 = message_parts.iter().map(ContextPart::tokens).sum();
        let mut message_parts = largest_first(message_parts);
        message_parts.push(ContextPart::Unattributed {
            tokens: messages.saturating_sub(attributed),
        });
        let instruction_parts = self
            .instructions
            .into_iter()
            .map(|(entry, tokens)| ContextPart::Instruction { entry, tokens })
            .collect();
        let source_parts = self
            .sources
            .into_iter()
            .map(|(source, tokens)| ContextPart::Source { source, tokens })
            .collect();
        let estimated = |kind, tokens, parts: Vec<ContextPart>| ContextRow {
            kind,
            tokens,
            estimated: true,
            parts: parts.into_iter().filter(|part| part.tokens() > 0).collect(),
        };
        let mut rows = vec![
            estimated(ContextRowKind::Messages, messages, message_parts),
            estimated(
                ContextRowKind::SystemPrompt,
                system_prompt_tokens,
                self.system_prompt,
            ),
            estimated(
                ContextRowKind::Commands,
                self.commands_tokens,
                largest_first(self.commands),
            ),
            estimated(
                ContextRowKind::Instructions,
                self.instructions_tokens,
                instruction_parts,
            ),
            estimated(
                ContextRowKind::Skills,
                self.skills_tokens,
                largest_first(self.skills),
            ),
            estimated(
                ContextRowKind::OtherContext,
                sources_tokens,
                largest_first(source_parts),
            ),
            estimated(ContextRowKind::Summary, self.summary, Vec::new()),
        ];
        if let Some((window, output_limit)) = window {
            let buffer = window - token_threshold(window, output_limit);
            let filled = above + messages;
            for (kind, tokens) in [
                (ContextRowKind::CompactionBuffer, buffer),
                (
                    ContextRowKind::FreeSpace,
                    window.saturating_sub(filled + buffer),
                ),
            ] {
                rows.push(ContextRow {
                    kind,
                    tokens,
                    estimated: false,
                    parts: Vec::new(),
                });
            }
        }
        rows.retain(|row| row.tokens > 0);
        rows
    }
}

/// `parts`, the largest first; parts of equal size keep their order.
fn largest_first(mut parts: Vec<ContextPart>) -> Vec<ContextPart> {
    parts.sort_by_key(|part| std::cmp::Reverse(part.tokens()));
    parts
}

/// Each skill of a catalog's `text` with the estimate of its entry.
fn skill_parts(text: &str) -> Vec<ContextPart> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<skill>") {
        let entry = &rest[start..];
        let end = entry.find("</skill>").map_or(entry.len(), |end| end + "</skill>".len());
        let entry = &entry[..end];
        let name = entry
            .split_once("<name>")
            .and_then(|(_, after)| after.split_once("</name>"))
            .map(|(name, _)| name.to_owned());
        if let Some(name) = name {
            parts.push(ContextPart::Skill {
                name,
                tokens: text_tokens(entry),
            });
        }
        rest = &rest[start + end..];
    }
    parts
}

/// The command group a call runs: the first of `groups` its script names
/// as a command, such as `demi browser`; `shell` for a script that runs
/// none; the tool's name for another tool.
fn call_group(call: &ToolCallBlock, groups: &[&str]) -> String {
    let input = tool_input(&call.input);
    let Some(script) = input.get("script").and_then(|script| script.as_str()) else {
        return call.tool_name.clone();
    };
    let words: Vec<&str> = script
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '`'))
        .filter(|word| !word.is_empty())
        .collect();
    for (index, word) in words.iter().enumerate() {
        if let Some(next) = words.get(index + 1) {
            let path = format!("{word} {next}");
            if groups.contains(&path.as_str()) {
                return path;
            }
        }
        if groups.contains(word) {
            return (*word).to_owned();
        }
    }
    SHELL_GROUP.to_owned()
}
