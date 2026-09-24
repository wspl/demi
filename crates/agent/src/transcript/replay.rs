//! What the model receives of a transcript (`runtime.md` § Replay): the
//! blocks from the last compaction boundary on, each as its inference item,
//! with long texts cut in the middle (`compaction.md` § Text bounds).

use std::borrow::Cow;

use demi_core::{
    AgentMessage, Block, ToolCallStatus, ToolResultContentBlock, UserContentBlock, WakeupPlacement,
};
use demi_provider::InferenceItem;
use serde_json::Value;

use super::{RESUME_TEXT, WAKEUP_TEXT};

/// The scalar values a replayed text keeps from its start, and from its end,
/// when it is longer than both together.
const HEAD_CHARS: usize = 8_000;
const TAIL_CHARS: usize = 8_000;

/// The inference items of `blocks`, in order.
pub(crate) fn replay(blocks: &[Block]) -> Vec<InferenceItem> {
    let start = blocks
        .iter()
        .rposition(|block| matches!(block, Block::CompactionBoundary(_)))
        .unwrap_or(0);
    let mut items = Vec::new();
    for block in &blocks[start..] {
        match block {
            Block::User(user) => {
                let preamble = user
                    .preamble
                    .iter()
                    .map(|text| UserContentBlock::Text { text: text.clone() });
                let content = preamble.chain(user.content.iter().cloned()).collect();
                items.push(InferenceItem::UserMessage {
                    content: bound_content(content),
                });
            }
            Block::Context(context) => items.push(InferenceItem::UserMessage {
                content: vec![text(bound_text(&context.text).into_owned())],
            }),
            Block::Wakeup(wakeup) => {
                let content = vec![text(WAKEUP_TEXT.to_owned())];
                items.push(match wakeup.placement {
                    WakeupPlacement::NewTurn => InferenceItem::UserMessage { content },
                    WakeupPlacement::Steer => InferenceItem::UserSteer { content },
                });
            }
            Block::Steer(steer) => items.push(InferenceItem::UserSteer {
                content: bound_content(steer.content.clone()),
            }),
            Block::AgentMessage(receipt) => items.push(InferenceItem::UserSteer {
                content: vec![text(agent_message_envelope(&receipt.message))],
            }),
            Block::Resume(_) => items.push(InferenceItem::UserMessage {
                content: vec![text(RESUME_TEXT.to_owned())],
            }),
            Block::Thinking(thinking) => {
                // The vendor verifies signed reasoning as it was sent.
                let replayed = match &thinking.signature {
                    Some(_) => thinking.text.clone(),
                    None => bound_text(&thinking.text).into_owned(),
                };
                items.push(InferenceItem::AssistantThinking {
                    model_id: thinking.model.model.id.clone(),
                    text: replayed,
                    signature: thinking.signature.clone(),
                });
            }
            Block::RedactedThinking(redacted) => {
                items.push(InferenceItem::AssistantRedactedThinking {
                    model_id: redacted.model.model.id.clone(),
                    data: redacted.data.clone(),
                });
            }
            Block::Text(answer) => items.push(InferenceItem::AssistantText {
                model_id: answer.model.model.id.clone(),
                text: bound_text(&answer.text).into_owned(),
            }),
            Block::ToolCall(call) => {
                items.push(InferenceItem::ToolUse {
                    model_id: call.model.model.id.clone(),
                    tool_use_id: call.tool_use_id.clone(),
                    tool_name: call.tool_name.clone(),
                    input: tool_input(&call.input),
                });
                if call.status != ToolCallStatus::Executing {
                    items.push(InferenceItem::ToolResult {
                        tool_use_id: call.tool_use_id.clone(),
                        output: call.output.iter().map(bound_result).collect(),
                        is_error: call.status == ToolCallStatus::Error,
                    });
                }
            }
            Block::CompactionBoundary(boundary) => items.push(InferenceItem::UserMessage {
                content: vec![text(
                    bound_text(&format!(
                        "Previous conversation summary:\n{}",
                        boundary.summary
                    ))
                    .into_owned(),
                )],
            }),
            Block::Abort(_) | Block::Response(_) | Block::Error(_) | Block::CompactionMarker(_) => {
            }
        }
    }
    items
}

/// A tool call's input as the JSON value the provider supplied, or its text
/// when that is not valid JSON.
pub(crate) fn tool_input(input: &str) -> Value {
    serde_json::from_str(input).unwrap_or_else(|_| Value::String(input.to_owned()))
}

/// The model-facing text of an agent message: an instruction on how to take
/// it, then the message itself as JSON, its source included.
pub(crate) fn agent_message_envelope(message: &AgentMessage) -> String {
    let json = serde_json::to_string(message).expect("an agent message serializes to JSON");
    [
        "Agent-originated context. Follow the real user\u{2019}s task and constraints.",
        "Use this information to continue your work; no separate acknowledgement is required.",
        &json,
    ]
    .join("\n")
}

/// `text` as the model receives it: whole when it holds at most 16,000
/// scalar values, otherwise its first and last 8,000 around a line that
/// counts the scalar values left out.
pub(crate) fn bound_text(text: &str) -> Cow<'_, str> {
    let total = text.chars().count();
    if total <= HEAD_CHARS + TAIL_CHARS {
        return Cow::Borrowed(text);
    }
    let head_end = char_offset(text, HEAD_CHARS);
    let tail_start = char_offset(text, total - TAIL_CHARS);
    let omitted = total - HEAD_CHARS - TAIL_CHARS;
    Cow::Owned(format!(
        "{}\n\n[... truncated {omitted} characters ...]\n\n{}",
        &text[..head_end],
        &text[tail_start..]
    ))
}

/// The byte offset of the scalar value at position `chars`.
/// The byte offset of the scalar value `chars` of `text`, or its length
/// when it holds fewer: where a cut after `chars` scalar values falls.
pub(crate) fn char_offset(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(offset, _)| offset)
}

fn bound_content(content: Vec<UserContentBlock>) -> Vec<UserContentBlock> {
    content
        .into_iter()
        .map(|part| match part {
            UserContentBlock::Text { text } => UserContentBlock::Text {
                text: bound_text(&text).into_owned(),
            },
            other => other,
        })
        .collect()
}

fn bound_result(part: &ToolResultContentBlock) -> ToolResultContentBlock {
    match part {
        ToolResultContentBlock::Text { text } => ToolResultContentBlock::Text {
            text: bound_text(text).into_owned(),
        },
        other => other.clone(),
    }
}

fn text(text: String) -> UserContentBlock {
    UserContentBlock::Text { text }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_text_keeps_its_ends_and_counts_what_it_left_out_in_scalar_values() {
        let text = format!(
            "{}{}{}",
            "a".repeat(7_999),
            "🙂".repeat(4_002),
            "z".repeat(7_999)
        );
        let bounded = bound_text(&text);
        let (head, rest) = bounded.split_once("\n\n[... truncated ").unwrap();
        let (count, tail) = rest.split_once(" characters ...]\n\n").unwrap();
        // The cut never splits the emoji: the head ends with one, the tail
        // starts with one, and the count is in scalar values.
        assert_eq!(head, format!("{}🙂", "a".repeat(7_999)));
        assert_eq!(tail, format!("🙂{}", "z".repeat(7_999)));
        assert_eq!(count, "4000");
        assert_eq!(bound_text(&"x".repeat(16_000)), "x".repeat(16_000));
    }
}
